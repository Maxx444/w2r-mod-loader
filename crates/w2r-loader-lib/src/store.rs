//! The manager's view of the w2r root (docs/layout.md): paths, `state.json` access, verification
//! of what is installed. Every plugin id is checked against the manifest id rule before it is
//! joined to a path.

use crate::Error;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use w2r_formats::manifest::{self, is_valid_id};
use w2r_formats::state::{self, State};

/// The `w2r` folder (`%USERPROFILE%\Saved Games\Warcraft2Remastered\w2r`, or `W2R_ROOT`).
#[derive(Clone, Debug)]
pub struct Root(pub PathBuf);

/// A plugin id that passed the manifest id rule.
#[derive(Clone, Copy, Debug)]
pub struct Id<'a>(&'a str);

impl<'a> Id<'a> {
    pub fn new(s: &'a str) -> Result<Id<'a>, Error> {
        if is_valid_id(s) {
            Ok(Id(s))
        } else {
            Err(Error::new(
                "bad_id",
                format!("{s:?} is not a valid plugin id"),
            ))
        }
    }
    pub fn as_str(&self) -> &'a str {
        self.0
    }
}

impl Root {
    pub fn plugins_dir(&self) -> PathBuf {
        self.0.join("plugins")
    }
    pub fn plugin_dir(&self, id: Id) -> PathBuf {
        self.plugins_dir().join(id.0)
    }
    pub fn config_dir(&self) -> PathBuf {
        self.0.join("config")
    }
    pub fn state_path(&self) -> PathBuf {
        self.config_dir().join("state.json")
    }
    pub fn cache_dir(&self) -> PathBuf {
        self.0.join("cache")
    }
    pub fn staging_dir(&self, id: Id) -> PathBuf {
        self.cache_dir().join("staging").join(id.0)
    }
    pub fn rollback_dir(&self, id: Id) -> PathBuf {
        self.cache_dir().join("rollback").join(id.0)
    }

    /// A missing `state.json` is an empty state; a bad one is an error (the host would load
    /// nothing from it, so the manager must not rewrite it blindly).
    pub fn load_state(&self) -> Result<State, Error> {
        match read_bounded(&self.state_path(), state::MAX_BYTES) {
            Ok(raw) => state::parse(&raw).map_err(|r| Error::new("bad_state", r)),
            Err(e) if e.code == "not_found" => Ok(State::default()),
            Err(e) => Err(e),
        }
    }

    /// The author in an installed plugin's `plugin.json`; `None` when it is missing or invalid
    /// (display only: `verify` reports a bad manifest).
    pub fn installed_author(&self, id: &str) -> Option<String> {
        self.installed_manifest(id).map(|m| m.author)
    }

    /// An installed plugin's validated `plugin.json`; `None` when it is missing or invalid
    /// (display only: `verify` reports a bad manifest).
    pub fn installed_manifest(&self, id: &str) -> Option<manifest::Manifest> {
        let dir = self.plugin_dir(Id::new(id).ok()?);
        let raw = read_bounded(&dir.join("plugin.json"), manifest::MAX_BYTES).ok()?;
        manifest::validate(&raw).ok()
    }

    pub fn save_state(&self, state: &State) -> Result<(), Error> {
        fs::create_dir_all(self.config_dir())?;
        state::write(&self.state_path(), state)?;
        Ok(())
    }
}

/// Reads a regular file of at most `max` bytes. Code `not_found` when it does not exist.
pub fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>, Error> {
    let f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::new("not_found", path.display().to_string()));
        }
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    f.take(max as u64 + 1).read_to_end(&mut out)?;
    if out.len() > max {
        return Err(Error::new(
            "too_large",
            format!("{} is over {max} bytes", path.display()),
        ));
    }
    Ok(out)
}

pub fn sha256_file(path: &Path) -> Result<[u8; 32], Error> {
    let bytes = read_bounded(path, crate::package::MAX_UNCOMPRESSED as usize)?;
    Ok(Sha256::digest(&bytes).into())
}

/// One thing `verify` found wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub id: String,
    pub problem: String,
}

/// Re-checks every plugin `state.json` lists against what is on disk: the folder, a valid
/// `plugin.json` of the recorded version, the entry DLL against the pin, no stray files. Also
/// reports plugin folders that `state.json` does not list.
pub fn verify(root: &Root) -> Result<Vec<Finding>, Error> {
    let state = root.load_state()?;
    let mut out = Vec::new();
    let mut add = |id: &str, p: String| {
        out.push(Finding {
            id: id.to_owned(),
            problem: p,
        })
    };
    for p in &state.plugins {
        let id = Id::new(&p.id)?;
        let dir = root.plugin_dir(id);
        if !dir.is_dir() {
            add(&p.id, "folder is missing".into());
            continue;
        }
        let m = match read_bounded(&dir.join("plugin.json"), manifest::MAX_BYTES) {
            Ok(raw) => match manifest::validate(&raw) {
                Ok(m) => m,
                Err(r) => {
                    add(&p.id, format!("plugin.json is refused: {r}"));
                    continue;
                }
            },
            Err(e) => {
                add(&p.id, format!("plugin.json: {e}"));
                continue;
            }
        };
        if m.id != p.id || m.version != p.version {
            add(
                &p.id,
                "plugin.json is not the recorded id and version".into(),
            );
        }
        match sha256_file(&dir.join(&m.entry)) {
            Ok(h) if h == p.sha256 => {}
            Ok(_) => add(&p.id, format!("{} differs from the pinned hash", m.entry)),
            Err(e) => add(&p.id, format!("{}: {e}", m.entry)),
        }
        let allowed = [
            "plugin.json",
            m.entry.as_str(),
            "README.md",
            "LICENSE",
            "data",
        ];
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !allowed.contains(&name.as_str()) {
                    add(&p.id, format!("unexpected file {name:?}"));
                }
            }
        }
    }
    if let Ok(rd) = fs::read_dir(root.plugins_dir()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let only_data =
                fs::read_dir(e.path()).is_ok_and(|d| d.flatten().all(|x| x.file_name() == "data"));
            if state.get(&name).is_none() && !only_data {
                add(&name, "folder is not listed in state.json".into());
            }
        }
    }
    Ok(out)
}
