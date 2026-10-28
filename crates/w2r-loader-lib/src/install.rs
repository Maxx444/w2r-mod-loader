//! Install, update, rollback, remove, enable/disable (docs/manager.md, slice 3).
//!
//! An install extracts into `cache\staging\<id>`, moves the plugin's `data\` across, parks the
//! current version in `cache\rollback\<id>\`, renames the staged folder into `plugins\<id>` and
//! only then writes `state.json`. The host loads what `state.json` lists, so a crash between the
//! rename and the write leaves the old record describing a folder whose pin no longer matches
//! (refused at load, never run); `verify` reports it. A failure before the state write puts the
//! folders back.

use crate::Error;
use crate::index::VersionEntry;
use crate::package;
use crate::store::{Id, Root};
use std::fs;
use std::path::Path;
use w2r_formats::semver::Version;
use w2r_formats::state::{PluginState, State, parse as parse_state};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Installed,
    Updated { from: Version },
    Reinstalled,
}

/// Capabilities in the manifest that the user has not approved for this plugin yet. Empty means
/// the install needs no prompt (a fresh plugin with no capabilities, or an update that adds none).
pub fn pending_approvals(state: &State, entry: &VersionEntry) -> Vec<String> {
    let have = state
        .get(&entry.manifest.id)
        .map(|p| p.approved_capabilities.as_slice());
    entry
        .manifest
        .capabilities
        .iter()
        .filter(|c| !have.is_some_and(|h| h.contains(c)))
        .cloned()
        .collect()
}

/// The installed, enabled plugins that `manifest` replaces (its `replaces` list, matched by id and
/// version). Installing an enabled plugin disables these in the same `state.json` write; the host
/// also refuses them at load (`E_REPLACED`), so this keeps the plugin list honest rather than
/// being the only guard. Callers can use it to tell the user what is about to be switched off.
pub fn replaced_ids(state: &State, manifest: &w2r_formats::manifest::Manifest) -> Vec<String> {
    state
        .plugins
        .iter()
        .filter(|p| p.enabled && manifest.replaces_plugin(&p.id, &p.version))
        .map(|p| p.id.clone())
        .collect()
}

/// Installs or updates one plugin from a downloaded package.
///
/// `approved_now` is what the user just agreed to; every capability from `pending_approvals`
/// must be in it. `manager` is this manager's version (`min_manager` check).
pub fn install(
    root: &Root,
    entry: &VersionEntry,
    package_bytes: &[u8],
    approved_now: &[String],
    manager: &Version,
) -> Result<Outcome, Error> {
    if &entry.min_manager > manager {
        return Err(Error::new(
            "manager_too_old",
            format!("needs W2R Mod Loader {} or newer", entry.min_manager),
        ));
    }
    let id = Id::new(&entry.manifest.id)?;
    let mut state = root.load_state()?;
    let missing: Vec<String> = pending_approvals(&state, entry)
        .into_iter()
        .filter(|c| !approved_now.contains(c))
        .collect();
    if !missing.is_empty() {
        return Err(Error::new("approval_needed", missing.join(", ")));
    }

    let stage = root.staging_dir(id);
    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }
    fs::create_dir_all(stage.parent().unwrap_or(&stage))?;
    let extracted = package::extract(package_bytes, entry, &stage)?;

    let live = root.plugin_dir(id);
    let park = root.rollback_dir(id);
    let previous = state.get(id.as_str()).cloned();
    let outcome = match &previous {
        None => Outcome::Installed,
        Some(p) if p.version == entry.version => Outcome::Reinstalled,
        Some(p) => Outcome::Updated {
            from: p.version.clone(),
        },
    };

    let swap = Swap::begin(&live, &stage, &park, previous.as_ref())?;
    let new_state = PluginState {
        id: id.as_str().to_owned(),
        version: entry.version.clone(),
        enabled: previous.as_ref().is_none_or(|p| p.enabled),
        sha256: extracted.entry_sha256,
        approved_capabilities: entry.manifest.capabilities.clone(),
    };
    let enabled = new_state.enabled;
    upsert(&mut state, new_state);
    if enabled {
        for old in replaced_ids(&state, &entry.manifest) {
            if let Some(p) = state.plugins.iter_mut().find(|p| p.id == old) {
                p.enabled = false;
            }
        }
    }
    match root.save_state(&state) {
        Ok(()) => {
            swap.commit();
            Ok(outcome)
        }
        Err(e) => {
            swap.undo();
            Err(e)
        }
    }
}

fn upsert(state: &mut State, p: PluginState) {
    match state.plugins.iter_mut().find(|e| e.id == p.id) {
        Some(slot) => *slot = p,
        None => state.plugins.push(p),
    }
}

/// The folder moves of an install, reversible until `commit`.
struct Swap<'a> {
    live: &'a Path,
    stage: &'a Path,
    park: &'a Path,
    parked: bool,
    moved_data: bool,
}

impl<'a> Swap<'a> {
    fn begin(
        live: &'a Path,
        stage: &'a Path,
        park: &'a Path,
        previous: Option<&PluginState>,
    ) -> Result<Swap<'a>, Error> {
        let mut s = Swap {
            live,
            stage,
            park,
            parked: false,
            moved_data: false,
        };
        if let Err(e) = s.run(previous) {
            s.undo();
            return Err(e);
        }
        Ok(s)
    }

    fn run(&mut self, previous: Option<&PluginState>) -> Result<(), Error> {
        if let Some(parent) = self.live.parent() {
            fs::create_dir_all(parent)?;
        }
        if self.live.exists() {
            let data = self.live.join("data");
            if data.is_dir() {
                fs::rename(&data, self.stage.join("data"))?;
                self.moved_data = true;
            }
            if self.park.exists() {
                fs::remove_dir_all(self.park)?;
            }
            fs::create_dir_all(self.park)?;
            fs::rename(self.live, self.park.join("plugin"))?;
            self.parked = true;
            if let Some(p) = previous {
                let one = State {
                    plugins: vec![p.clone()],
                };
                fs::write(self.park.join("state.json"), one.to_json())?;
            }
        }
        fs::rename(self.stage, self.live)?;
        Ok(())
    }

    fn commit(self) {}

    /// Best effort: put every folder back where it was.
    fn undo(self) {
        if self.live.exists() && !self.stage.exists() {
            let _ = fs::rename(self.live, self.stage);
        }
        if self.parked {
            let _ = fs::rename(self.park.join("plugin"), self.live);
            let _ = fs::remove_dir_all(self.park);
        }
        if self.moved_data {
            let _ = fs::rename(self.stage.join("data"), self.live.join("data"));
        }
        let _ = fs::remove_dir_all(self.stage);
    }
}

/// Swaps the plugin with the version parked by the last install or rollback. Calling it twice
/// returns to where it started. The `enabled` flag stays as it is now.
pub fn rollback(root: &Root, id: &str) -> Result<Version, Error> {
    let id = Id::new(id)?;
    let park = root.rollback_dir(id);
    let parked_dir = park.join("plugin");
    let parked_state = match crate::store::read_bounded(&park.join("state.json"), 64 * 1024) {
        Ok(raw) => parse_state(&raw).map_err(|r| Error::new("bad_state", r))?,
        Err(e) if e.code == "not_found" => {
            return Err(Error::new(
                "no_rollback",
                format!("{id:?} has no parked version"),
            ));
        }
        Err(e) => return Err(e),
    };
    let target = parked_state
        .plugins
        .into_iter()
        .find(|p| p.id == id.as_str())
        .ok_or_else(|| Error::new("no_rollback", "parked record names another plugin"))?;

    // The parked files must still match the pin they were installed with.
    let m = w2r_formats::manifest::validate(&crate::store::read_bounded(
        &parked_dir.join("plugin.json"),
        w2r_formats::manifest::MAX_BYTES,
    )?)
    .map_err(|r| Error::new("bad_manifest", r))?;
    if m.id != id.as_str() || m.version != target.version {
        return Err(Error::new(
            "bad_manifest",
            "the parked plugin.json is not the recorded id and version",
        ));
    }
    if crate::store::sha256_file(&parked_dir.join(&m.entry))? != target.sha256 {
        return Err(Error::new(
            "hash_mismatch",
            "the parked version was modified",
        ));
    }

    let mut state = root.load_state()?;
    let current = state
        .get(id.as_str())
        .cloned()
        .ok_or_else(|| Error::new("not_installed", id.as_str().to_owned()))?;
    let live = root.plugin_dir(id);
    let tmp = park.join("swap");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    let data = live.join("data");
    let had_data = data.is_dir();
    fs::rename(&live, &tmp)?;
    let restore = |e: Error| {
        let _ = fs::rename(&tmp, &live);
        e
    };
    fs::rename(&parked_dir, &live).map_err(|e| restore(e.into()))?;
    if had_data {
        let _ = fs::rename(tmp.join("data"), live.join("data"));
    }
    fs::rename(&tmp, &parked_dir)?;
    fs::write(
        park.join("state.json"),
        State {
            plugins: vec![current.clone()],
        }
        .to_json(),
    )?;

    let from = target.version.clone();
    upsert(
        &mut state,
        PluginState {
            enabled: current.enabled,
            ..target
        },
    );
    root.save_state(&state)?;
    Ok(from)
}

/// Flips `enabled`. Errors when the plugin is not installed.
pub fn set_enabled(root: &Root, id: &str, enabled: bool) -> Result<(), Error> {
    let id = Id::new(id)?;
    let mut state = root.load_state()?;
    if !state.set_enabled(id.as_str(), enabled) {
        return Err(Error::new("not_installed", id.as_str().to_owned()));
    }
    root.save_state(&state)
}

/// Removes the plugin from `state.json` first (so the host stops loading it), then deletes its
/// folder and parked version. `data\` is kept unless `purge_data`.
pub fn remove(root: &Root, id: &str, purge_data: bool) -> Result<(), Error> {
    let id = Id::new(id)?;
    let mut state = root.load_state()?;
    let before = state.plugins.len();
    state.plugins.retain(|p| p.id != id.as_str());
    if state.plugins.len() == before {
        return Err(Error::new("not_installed", id.as_str().to_owned()));
    }
    root.save_state(&state)?;
    let live = root.plugin_dir(id);
    if purge_data {
        let _ = fs::remove_dir_all(&live);
    } else if let Ok(rd) = fs::read_dir(&live) {
        for e in rd.flatten() {
            if e.file_name() != "data" {
                let p = e.path();
                let _ = if p.is_dir() {
                    fs::remove_dir_all(&p)
                } else {
                    fs::remove_file(&p)
                };
            }
        }
    }
    let _ = fs::remove_dir_all(root.rollback_dir(id));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{self, Finding};
    use crate::testutil::{ENTRY, TempRoot, package};

    fn mgr() -> Version {
        Version::parse("0.1.0").unwrap()
    }

    fn caps(c: &[&str]) -> Vec<String> {
        c.iter().map(|s| s.to_string()).collect()
    }

    fn id() -> Id<'static> {
        Id::new("example-plugin").unwrap()
    }

    #[test]
    fn replaced_ids_lists_enabled_matching_plugins_only() {
        use w2r_formats::manifest::{Dependency, validate};
        use w2r_formats::semver::VersionReq;

        let (mut e, _) = package("1.0.0", &[], b"dll");
        e.manifest.replaces = vec![Dependency {
            id: "old-plugin".into(),
            version_req: VersionReq::parse("<1.0.0").unwrap(),
        }];
        let _ = validate; // the parser side is covered by docs/vectors/manifest.json
        let plugin = |id: &str, version: &str, enabled: bool| PluginState {
            id: id.into(),
            version: Version::parse(version).unwrap(),
            enabled,
            sha256: [0; 32],
            approved_capabilities: vec![],
        };
        let mut state = State::default();
        state.plugins = vec![
            plugin("old-plugin", "0.2.0", true),
            plugin("other-plugin", "0.2.0", true),
        ];
        assert_eq!(replaced_ids(&state, &e.manifest), vec!["old-plugin"]);
        // Out of the version range, or already disabled: left alone.
        state.plugins[0].version = Version::parse("1.0.0").unwrap();
        assert!(replaced_ids(&state, &e.manifest).is_empty());
        state.plugins[0].version = Version::parse("0.2.0").unwrap();
        state.plugins[0].enabled = false;
        assert!(replaced_ids(&state, &e.manifest).is_empty());
    }

    #[test]
    fn fresh_install_pins_and_verifies() {
        let t = TempRoot::new("fresh");
        let (e, z) = package("1.0.0", &["game.read"], b"dll-1");
        assert_eq!(
            install(&t.0, &e, &z, &[], &mgr()).unwrap_err().code,
            "approval_needed"
        );
        assert!(!t.0.plugin_dir(id()).exists());
        let out = install(&t.0, &e, &z, &caps(&["game.read"]), &mgr()).unwrap();
        assert_eq!(out, Outcome::Installed);
        let st = t.0.load_state().unwrap();
        let p = st.get("example-plugin").unwrap();
        assert!(p.enabled);
        assert_eq!(p.approved_capabilities, caps(&["game.read"]));
        assert_eq!(
            p.sha256,
            store::sha256_file(&t.0.plugin_dir(id()).join(ENTRY)).unwrap()
        );
        assert_eq!(store::verify(&t.0).unwrap(), vec![]);
    }

    #[test]
    fn update_keeps_enabled_and_data_and_asks_only_for_new_capabilities() {
        let t = TempRoot::new("update");
        let (e1, z1) = package("1.0.0", &["game.read"], b"dll-1");
        install(&t.0, &e1, &z1, &caps(&["game.read"]), &mgr()).unwrap();
        set_enabled(&t.0, "example-plugin", false).unwrap();
        let data = t.0.plugin_dir(id()).join("data");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("log.txt"), "kept").unwrap();

        let (e2, z2) = package("1.1.0", &["game.read", "chat.write"], b"dll-2");
        assert_eq!(
            pending_approvals(&t.0.load_state().unwrap(), &e2),
            caps(&["chat.write"])
        );
        assert_eq!(
            install(&t.0, &e2, &z2, &[], &mgr()).unwrap_err().code,
            "approval_needed"
        );
        let out = install(&t.0, &e2, &z2, &caps(&["chat.write"]), &mgr()).unwrap();
        assert!(matches!(out, Outcome::Updated { ref from } if *from == e1.version));
        let st = t.0.load_state().unwrap();
        assert!(
            !st.get("example-plugin").unwrap().enabled,
            "a disabled plugin stays disabled"
        );
        assert_eq!(fs::read_to_string(data.join("log.txt")).unwrap(), "kept");
        assert_eq!(store::verify(&t.0).unwrap(), vec![]);

        // An update that drops a capability needs no prompt and shrinks the approval.
        let (e3, z3) = package("1.2.0", &["game.read"], b"dll-3");
        assert!(pending_approvals(&t.0.load_state().unwrap(), &e3).is_empty());
        install(&t.0, &e3, &z3, &[], &mgr()).unwrap();
        let st = t.0.load_state().unwrap();
        assert_eq!(
            st.get("example-plugin").unwrap().approved_capabilities,
            caps(&["game.read"])
        );
    }

    #[test]
    fn rollback_swaps_back_and_forth() {
        let t = TempRoot::new("rollback");
        let (e1, z1) = package("1.0.0", &[], b"dll-1");
        let (e2, z2) = package("2.0.0", &[], b"dll-2");
        install(&t.0, &e1, &z1, &[], &mgr()).unwrap();
        assert_eq!(
            rollback(&t.0, "example-plugin").unwrap_err().code,
            "no_rollback"
        );
        install(&t.0, &e2, &z2, &[], &mgr()).unwrap();
        assert_eq!(rollback(&t.0, "example-plugin").unwrap(), e1.version);
        let st = t.0.load_state().unwrap();
        assert_eq!(st.get("example-plugin").unwrap().version, e1.version);
        assert_eq!(
            fs::read(t.0.plugin_dir(id()).join(ENTRY)).unwrap(),
            b"dll-1"
        );
        assert_eq!(store::verify(&t.0).unwrap(), vec![]);
        assert_eq!(rollback(&t.0, "example-plugin").unwrap(), e2.version);
        assert_eq!(
            fs::read(t.0.plugin_dir(id()).join(ENTRY)).unwrap(),
            b"dll-2"
        );
        assert_eq!(store::verify(&t.0).unwrap(), vec![]);
    }

    #[test]
    fn rollback_refuses_a_modified_parked_version() {
        let t = TempRoot::new("rbmod");
        let (e1, z1) = package("1.0.0", &[], b"dll-1");
        let (e2, z2) = package("2.0.0", &[], b"dll-2");
        install(&t.0, &e1, &z1, &[], &mgr()).unwrap();
        install(&t.0, &e2, &z2, &[], &mgr()).unwrap();
        fs::write(t.0.rollback_dir(id()).join("plugin").join(ENTRY), b"evil").unwrap();
        assert_eq!(
            rollback(&t.0, "example-plugin").unwrap_err().code,
            "hash_mismatch"
        );
        assert_eq!(
            fs::read(t.0.plugin_dir(id()).join(ENTRY)).unwrap(),
            b"dll-2"
        );
    }

    #[test]
    fn rollback_refuses_a_parked_manifest_for_another_version() {
        let t = TempRoot::new("rbmanifest");
        let (e1, z1) = package("1.0.0", &[], b"dll-1");
        let (e2, z2) = package("2.0.0", &[], b"dll-2");
        install(&t.0, &e1, &z1, &[], &mgr()).unwrap();
        install(&t.0, &e2, &z2, &[], &mgr()).unwrap();
        // A valid manifest, but not the recorded version of the parked plugin.
        let other = crate::testutil::manifest_json("9.9.9", &[]);
        let parked = t.0.rollback_dir(id()).join("plugin").join("plugin.json");
        fs::write(parked, other).unwrap();
        assert_eq!(
            rollback(&t.0, "example-plugin").unwrap_err().code,
            "bad_manifest"
        );
        assert_eq!(
            fs::read(t.0.plugin_dir(id()).join(ENTRY)).unwrap(),
            b"dll-2"
        );
    }

    #[test]
    fn a_refused_package_changes_nothing() {
        let t = TempRoot::new("refused");
        let (e1, z1) = package("1.0.0", &[], b"dll-1");
        install(&t.0, &e1, &z1, &[], &mgr()).unwrap();
        let before = fs::read_to_string(t.0.state_path()).unwrap();
        let (e2, mut z2) = package("2.0.0", &[], b"dll-2");
        let last = z2.len() - 1;
        z2[last] ^= 1;
        assert_eq!(
            install(&t.0, &e2, &z2, &[], &mgr()).unwrap_err().code,
            "hash_mismatch"
        );
        assert_eq!(fs::read_to_string(t.0.state_path()).unwrap(), before);
        assert_eq!(
            fs::read(t.0.plugin_dir(id()).join(ENTRY)).unwrap(),
            b"dll-1"
        );
        assert!(!t.0.staging_dir(id()).exists());
    }

    #[test]
    fn manager_too_old_is_refused() {
        let t = TempRoot::new("old");
        let (mut e, z) = package("1.0.0", &[], b"dll");
        e.min_manager = Version::parse("9.0.0").unwrap();
        assert_eq!(
            install(&t.0, &e, &z, &[], &mgr()).unwrap_err().code,
            "manager_too_old"
        );
    }

    #[test]
    fn enable_disable_remove() {
        let t = TempRoot::new("remove");
        let (e, z) = package("1.0.0", &[], b"dll");
        assert_eq!(
            set_enabled(&t.0, "example-plugin", true).unwrap_err().code,
            "not_installed"
        );
        install(&t.0, &e, &z, &[], &mgr()).unwrap();
        set_enabled(&t.0, "example-plugin", false).unwrap();
        assert!(
            !t.0.load_state()
                .unwrap()
                .get("example-plugin")
                .unwrap()
                .enabled
        );
        let data = t.0.plugin_dir(id()).join("data");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("x"), "d").unwrap();
        remove(&t.0, "example-plugin", false).unwrap();
        assert!(t.0.load_state().unwrap().plugins.is_empty());
        assert!(!t.0.plugin_dir(id()).join(ENTRY).exists());
        assert!(data.join("x").exists(), "data is kept");
        assert_eq!(
            store::verify(&t.0).unwrap(),
            vec![],
            "a data-only folder is not a finding"
        );
        assert_eq!(
            remove(&t.0, "example-plugin", false).unwrap_err().code,
            "not_installed"
        );
        install(&t.0, &e, &z, &[], &mgr()).unwrap();
        assert!(data.join("x").exists(), "reinstall finds its data again");
        remove(&t.0, "example-plugin", true).unwrap();
        assert!(!t.0.plugin_dir(id()).exists());
    }

    #[test]
    fn verify_reports_tampering_and_strays() {
        let t = TempRoot::new("verify");
        let (e, z) = package("1.0.0", &[], b"dll");
        install(&t.0, &e, &z, &[], &mgr()).unwrap();
        let dir = t.0.plugin_dir(id());
        fs::write(dir.join(ENTRY), b"swapped").unwrap();
        fs::write(dir.join("extra.dll"), b"x").unwrap();
        fs::create_dir_all(t.0.plugins_dir().join("stranger")).unwrap();
        fs::write(t.0.plugins_dir().join("stranger").join("a.dll"), b"x").unwrap();
        let f: Vec<Finding> = store::verify(&t.0).unwrap();
        let text: Vec<String> = f
            .iter()
            .map(|x| format!("{}: {}", x.id, x.problem))
            .collect();
        assert!(
            text.iter()
                .any(|s| s.contains("differs from the pinned hash")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|s| s.contains("unexpected file \"extra.dll\"")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|s| s.starts_with("stranger: folder is not listed")),
            "{text:?}"
        );
    }

    #[test]
    fn a_bad_state_file_is_never_rewritten() {
        let t = TempRoot::new("badstate");
        fs::create_dir_all(t.0.config_dir()).unwrap();
        fs::write(t.0.state_path(), "{not json").unwrap();
        let (e, z) = package("1.0.0", &[], b"dll");
        assert_eq!(
            install(&t.0, &e, &z, &[], &mgr()).unwrap_err().code,
            "bad_state"
        );
        assert_eq!(fs::read_to_string(t.0.state_path()).unwrap(), "{not json");
    }
}
