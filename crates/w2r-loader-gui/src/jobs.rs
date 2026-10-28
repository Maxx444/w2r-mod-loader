//! What the window can ask for, run off the UI thread, and the snapshot it shows afterwards.
//! No UI code here, so tests drive it against a signed test repository. All real work is in
//! `w2r-loader-lib`; this file only sequences calls (docs/manager.md).

use std::path::PathBuf;
use w2r_formats::manifest::Dependency;
use w2r_formats::semver::Version;
use w2r_loader_lib::index::{Index, VersionEntry};
use w2r_loader_lib::session::{self, Session};
use w2r_loader_lib::store::{self, Finding, Root};
use w2r_loader_lib::coremod::{self, Installed as CoreInstalled};
use w2r_loader_lib::{Error, catalog, install, mgrupdate};

#[derive(Clone, Debug)]
pub enum Job {
    /// Fetch and verify a fresh index.
    Refresh,
    /// Install `version`, or the latest installable one, with the capabilities the user ticked.
    Install {
        id: String,
        version: Option<Version>,
        approved: Vec<String>,
    },
    Remove {
        id: String,
        purge: bool,
    },
    SetEnabled {
        id: String,
        on: bool,
    },
    Rollback {
        id: String,
    },
    /// Re-check installed files against their pins (also done after every other job).
    Verify,
    /// Fetch `core.json` to see what is published.
    CoreCheck,
    /// Install or update the core mod (`version.dll`); the game must be closed.
    CoreUpdate,
    /// `CoreUpdate`, also replacing another mod's `version.dll` (moved to a backup, never deleted).
    CoreReplace,
    /// Swap the previous core back in (again to swap forward).
    CoreRollback,
    /// Remember the folder the user picked as the game folder.
    CoreGameDir { dir: PathBuf },
    /// Replace this exe with the newer manager `manager.json` offers; the app restarts after.
    SelfUpdate { latest: mgrupdate::Latest },
}

pub fn run(session: &Session, job: &Job) -> Result<String, Error> {
    run_with_progress(session, job, &mut |_, _| {})
}

/// `run`, reporting `(bytes so far, total)` while a package downloads.
pub fn run_with_progress(
    session: &Session,
    job: &Job,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<String, Error> {
    match job {
        Job::Refresh => {
            let idx = session.refresh()?;
            Ok(format!(
                "index serial {}: {} plugins",
                idx.serial,
                idx.plugins.len()
            ))
        }
        Job::Install {
            id,
            version,
            approved,
        } => {
            let idx = session.index()?;
            let p = idx
                .plugin(id)
                .ok_or_else(|| Error::new("unknown_plugin", id.clone()))?;
            let entry = match version {
                Some(v) => catalog::exact(p, v),
                None => catalog::latest(p, &session.manager),
            }
            .ok_or_else(|| Error::new("no_version", format!("no installable version of {id}")))?;
            let v = &entry.version;
            Ok(match session.install_with_progress(entry, approved, progress)? {
                install::Outcome::Installed => format!("installed {id} {v}"),
                install::Outcome::Updated { from } => format!("updated {id} {from} -> {v}"),
                install::Outcome::Reinstalled => format!("reinstalled {id} {v}"),
            })
        }
        Job::Remove { id, purge } => {
            install::remove(&session.root, id, *purge).map_err(session::hint_io)?;
            Ok(format!("removed {id}"))
        }
        Job::SetEnabled { id, on } => {
            install::set_enabled(&session.root, id, *on)?;
            Ok(format!("{} {id}", if *on { "enabled" } else { "disabled" }))
        }
        Job::Rollback { id } => {
            let v = install::rollback(&session.root, id).map_err(session::hint_io)?;
            Ok(format!("{id} is now {v}"))
        }
        Job::CoreCheck => {
            let info = session.core_refresh()?;
            Ok(format!("newest core: {}", info.version))
        }
        Job::CoreUpdate | Job::CoreReplace => {
            let replace = matches!(job, Job::CoreReplace);
            let game = coremod::resolve_game_dir(&session.root, None)?;
            let now = coremod::installed(&game)?;
            let info = session.core_refresh()?;
            let replacing_other = replace && now == CoreInstalled::Foreign;
            if !replacing_other && !coremod::update_available(&now, &info, &session.manager) {
                return match now {
                    CoreInstalled::Foreign => Err(Error::new(
                        "not_ours",
                        "version.dll belongs to another mod; it is left alone",
                    )),
                    _ => Ok("the core is up to date".to_owned()),
                };
            }
            let out = session
                .core_install(&game, &info, replace, progress)
                .map_err(session::hint_io)?;
            coremod::save_game_dir(&session.root, &game)?;
            Ok(match &out.backed_up {
                Some(b) => format!(
                    "core is now {}; the other mod's version.dll was kept as {}",
                    core_version(&out.to),
                    b.display()
                ),
                None => format!("core is now {}", core_version(&out.to)),
            })
        }
        Job::CoreRollback => {
            let game = coremod::resolve_game_dir(&session.root, None)?;
            let out = coremod::rollback(&session.root, &game)?;
            Ok(format!("core is now {}", core_version(&out.to)))
        }
        Job::CoreGameDir { dir } => {
            let game = coremod::resolve_game_dir(&session.root, Some(dir))?;
            coremod::save_game_dir(&session.root, &game)?;
            Ok(format!("game folder: {}", game.display()))
        }
        Job::SelfUpdate { latest } => {
            let exe = std::env::current_exe()?;
            let v = mgrupdate::install_self(
                session.fetcher.as_ref(),
                &exe,
                latest,
                &session.manager,
                progress,
            )
            .map_err(session::hint_io)?;
            Ok(format!("W2R Mod Loader {v} is installed; restart to use it"))
        }
        Job::Verify => {
            let findings = store::verify(&session.root)?;
            Ok(if findings.is_empty() {
                "ok: every installed plugin matches its pin".to_owned()
            } else {
                format!("{} problem(s) found", findings.len())
            })
        }
    }
}

fn core_version(v: &Option<Version>) -> String {
    v.as_ref().map_or_else(|| "installed".to_owned(), |v| v.to_string())
}

/// The core mod as the settings tab shows it, from disk and the cached `core.json`.
#[derive(Clone, Debug)]
pub struct CoreView {
    /// The game folder, or why none is known.
    pub game_dir: Result<PathBuf, String>,
    pub installed: Option<CoreInstalled>,
    pub published: Option<Version>,
    pub update: bool,
}

impl CoreView {
    pub fn load(root: &Root, manager: &Version) -> CoreView {
        let game_dir = coremod::resolve_game_dir(root, None).map_err(|e| e.detail);
        let installed = game_dir
            .as_ref()
            .ok()
            .and_then(|d| coremod::installed(d).ok());
        let info = coremod::load_cached(root);
        let update = match (&installed, &info) {
            (Some(i), Some(info)) => coremod::update_available(i, info, manager),
            _ => false,
        };
        CoreView {
            game_dir,
            installed,
            published: info.map(|i| i.version),
            update,
        }
    }
}

/// An installed plugin as a row.
#[derive(Clone, Debug)]
pub struct Installed {
    pub id: String,
    pub version: Version,
    /// From the installed `plugin.json`.
    pub author: Option<String>,
    pub enabled: bool,
    pub capabilities: Vec<String>,
    /// The newer version the cached index offers, if any.
    pub update: Option<Version>,
    pub problem: Option<String>,
    /// The enabled installed plugin that replaces this one (`replaces` in its `plugin.json`); the
    /// host skips this plugin while that one loads.
    pub replaced_by: Option<String>,
}

/// Everything the window shows, loaded without the network.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub installed: Vec<Installed>,
    pub index: Option<Index>,
    /// Why the installed list or the index is not available.
    pub notes: Vec<String>,
    pub findings: Vec<Finding>,
    pub core: Option<CoreView>,
}

impl View {
    pub fn load(root: &Root, session: Option<&Session>) -> View {
        let mut v = View::default();
        if let Some(s) = session {
            v.core = Some(CoreView::load(root, &s.manager));
        }
        let state = match root.load_state() {
            Ok(s) => Some(s),
            Err(e) => {
                v.notes.push(format!("installed plugins: {e}"));
                None
            }
        };
        if let Some(s) = session {
            match s.cached_index() {
                Ok(i) => v.index = Some(i),
                Err(e) => v.notes.push(format!("no usable cached index ({e})")),
            }
        }
        match store::verify(root) {
            Ok(f) => v.findings = f,
            Err(e) if state.is_some() => v.notes.push(format!("verify: {e}")),
            Err(_) => {}
        }
        let updates = match (session, &v.index) {
            (Some(s), Some(idx)) => s.updates(idx).unwrap_or_default(),
            _ => Vec::new(),
        };
        for p in state.iter().flat_map(|s| &s.plugins) {
            v.installed.push(Installed {
                id: p.id.clone(),
                version: p.version.clone(),
                author: root.installed_author(&p.id),
                enabled: p.enabled,
                capabilities: p.approved_capabilities.clone(),
                update: updates
                    .iter()
                    .find(|(id, _)| *id == p.id)
                    .map(|(_, e)| e.version.clone()),
                problem: v
                    .findings
                    .iter()
                    .filter(|f| f.id == p.id)
                    .map(|f| f.problem.clone())
                    .reduce(|a, b| format!("{a}; {b}")),
                replaced_by: None,
            });
        }
        let replaces: Vec<(String, Vec<Dependency>)> = v
            .installed
            .iter()
            .filter(|p| p.enabled)
            .filter_map(|p| Some((p.id.clone(), root.installed_manifest(&p.id)?.replaces)))
            .filter(|(_, list)| !list.is_empty())
            .collect();
        mark_replaced(&mut v.installed, &replaces);
        v
    }
}

/// Sets `replaced_by` on each row that some plugin in `replaces` (id, its `replaces` list)
/// supersedes at the row's version.
pub fn mark_replaced(rows: &mut [Installed], replaces: &[(String, Vec<Dependency>)]) {
    for row in rows {
        row.replaced_by = replaces
            .iter()
            .find(|(by, list)| {
                *by != row.id
                    && list
                        .iter()
                        .any(|r| r.id == row.id && r.version_req.matches(&row.version))
            })
            .map(|(by, _)| by.clone());
    }
}

/// The root the settings name, or the default.
pub fn root_of(explicit: Option<&PathBuf>) -> Result<Root, Error> {
    session::resolve_root(explicit.map(|p| p.as_path()))
}

/// True while the game has `version.dll` loaded: Windows refuses a write-open of a file in use
/// (sharing violation, or mapped-image error), so updating the core would fail. Opens without
/// truncating and writes nothing.
pub fn game_running(game_dir: &std::path::Path) -> bool {
    let dll = game_dir.join("version.dll");
    match std::fs::OpenOptions::new().write(true).open(dll) {
        Ok(_) => false,
        Err(e) => matches!(e.raw_os_error(), Some(32 | 1224)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_or_missing_dll_is_not_running() {
        let dir = std::env::temp_dir().join(format!("w2r-loader-gui-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!game_running(&dir));
        std::fs::write(dir.join("version.dll"), b"x").unwrap();
        assert!(!game_running(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

impl Job {
    /// The plugin a job works on, for the spinner on its card.
    pub fn target(&self) -> Option<&str> {
        match self {
            Job::Install { id, .. }
            | Job::Remove { id, .. }
            | Job::SetEnabled { id, .. }
            | Job::Rollback { id } => Some(id),
            _ => None,
        }
    }
}

/// How one declared dependency stands, for the details and consent screens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepStatus {
    /// Installed and in range.
    Satisfied(Version),
    /// Installed and in range, but switched off (the host will not load it).
    Disabled(Version),
    /// Not installed yet, but part of the same install.
    Planned(Version),
    /// Installed, out of range.
    WrongVersion(Version),
    /// Not installed and not in this install.
    Missing,
}

pub fn dep_status(installed: &[Installed], planned: &[VersionEntry], d: &Dependency) -> DepStatus {
    match installed.iter().find(|i| i.id == d.id) {
        Some(i) if d.version_req.matches(&i.version) => {
            if i.enabled {
                DepStatus::Satisfied(i.version.clone())
            } else {
                DepStatus::Disabled(i.version.clone())
            }
        }
        _ => match planned
            .iter()
            .find(|e| e.manifest.id == d.id && d.version_req.matches(&e.version))
        {
            Some(e) => DepStatus::Planned(e.version.clone()),
            None => match installed.iter().find(|i| i.id == d.id) {
                Some(i) => DepStatus::WrongVersion(i.version.clone()),
                None => DepStatus::Missing,
            },
        },
    }
}

/// `wanted` plus every dependency it needs that the index can supply, each placed before what
/// needs it. Dependencies already installed in range are left alone.
pub fn with_dependencies(
    index: &Index,
    installed: &[Installed],
    manager: &Version,
    wanted: Vec<VersionEntry>,
) -> Vec<VersionEntry> {
    fn visit(
        index: &Index,
        installed: &[Installed],
        manager: &Version,
        entry: VersionEntry,
        out: &mut Vec<VersionEntry>,
        seen: &mut Vec<String>,
    ) {
        if seen.contains(&entry.manifest.id) {
            return;
        }
        seen.push(entry.manifest.id.clone());
        for d in &entry.manifest.dependencies {
            if matches!(
                dep_status(installed, out, d),
                DepStatus::Satisfied(_) | DepStatus::Disabled(_) | DepStatus::Planned(_)
            ) {
                continue;
            }
            let best = index.plugin(&d.id).and_then(|p| {
                p.versions
                    .iter()
                    .filter(|v| &v.min_manager <= manager && d.version_req.matches(&v.version))
                    .max_by(|a, b| a.version.cmp(&b.version))
            });
            if let Some(dep) = best {
                visit(index, installed, manager, dep.clone(), out, seen);
            }
        }
        out.push(entry);
    }
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for e in wanted {
        visit(index, installed, manager, e, &mut out, &mut seen);
    }
    out
}

/// Opens a folder or file in Explorer (the default program for a file).
pub fn open_path(path: &std::path::Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("{} does not exist yet", path.display()));
    }
    #[cfg(windows)]
    let opener = "explorer";
    #[cfg(not(windows))]
    let opener = "xdg-open";
    // Explorer reports failure through its exit code even when it worked, so only a failure to
    // start it counts.
    std::process::Command::new(opener)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {}: {e}", path.display()))
}

/// Starts the game through the Battle.net app (product code `W2R`, case-sensitive). Started directly the game has no
/// Battle.net connection, so no multiplayer. Falls back to the `battlenet://` link, which opens
/// the game's page in the app.
pub fn launch_game() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let found = ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"]
            .iter()
            .filter_map(|v| std::env::var_os(v))
            .map(|d| std::path::PathBuf::from(d).join("Battle.net").join("Battle.net.exe"))
            .find(|p| p.is_file());
        if let Some(exe) = found {
            return std::process::Command::new(exe)
                .raw_arg("--exec=\"launch W2R\"")
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("could not start Battle.net: {e}"));
        }
    }
    open_url("battlenet://W2R")
}

/// Starts the exe at this process's own path (the new one after a self-update). The caller
/// closes the window.
pub fn restart_self() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(&exe)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start {}: {e}", exe.display()))
}

/// Opens an `https` link in the default browser. Explorer gets the link as one argument (no
/// shell), and only plain `https` links are passed.
pub fn open_url(url: &str) -> Result<(), String> {
    if url != "battlenet://W2R" && !w2r_loader_lib::index::url_ok(url) {
        return Err(format!("not a link I will open: {url}"));
    }
    #[cfg(windows)]
    let opener = "explorer";
    #[cfg(not(windows))]
    let opener = "xdg-open";
    std::process::Command::new(opener)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {url}: {e}"))
}

#[cfg(test)]
mod dep_tests {
    use super::*;

    fn installed(id: &str, v: &str, enabled: bool) -> Installed {
        Installed {
            id: id.into(),
            version: Version::parse(v).unwrap(),
            author: None,
            enabled,
            capabilities: vec![],
            update: None,
            problem: None,
            replaced_by: None,
        }
    }

    #[test]
    fn mark_replaced_names_the_replacer_within_its_version_range() {
        let mut rows = vec![
            installed("old-plugin", "0.2.0", true),
            installed("kept-plugin", "1.0.0", true),
            installed("new-plugin", "1.0.0", true),
        ];
        let replaces = vec![(
            "new-plugin".to_string(),
            vec![dep("old-plugin", "<1.0.0"), dep("kept-plugin", "<1.0.0")],
        )];
        mark_replaced(&mut rows, &replaces);
        assert_eq!(rows[0].replaced_by.as_deref(), Some("new-plugin"));
        assert_eq!(rows[1].replaced_by, None, "1.0.0 is outside <1.0.0");
        assert_eq!(rows[2].replaced_by, None);
    }

    fn dep(id: &str, req: &str) -> Dependency {
        Dependency {
            id: id.into(),
            version_req: w2r_formats::semver::VersionReq::parse(req).unwrap(),
        }
    }

    #[test]
    fn status_of_each_kind_of_dependency() {
        let have = [installed("a", "1.2.0", true), installed("b", "1.0.0", false)];
        assert!(matches!(dep_status(&have, &[], &dep("a", ">=1.0.0")), DepStatus::Satisfied(_)));
        assert!(matches!(dep_status(&have, &[], &dep("b", ">=1.0.0")), DepStatus::Disabled(_)));
        assert!(matches!(dep_status(&have, &[], &dep("a", ">=2.0.0")), DepStatus::WrongVersion(_)));
        assert_eq!(dep_status(&have, &[], &dep("c", ">=1.0.0")), DepStatus::Missing);
    }
}
