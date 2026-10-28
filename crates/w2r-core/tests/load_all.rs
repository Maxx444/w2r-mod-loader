//! Load policy end to end: a temporary host root with `state.json` and plugin folders made from
//! the fixture DLLs, through checks 1 to 10.

#![cfg(windows)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::OnceLock;

use w2r_core::fingerprint;
use w2r_core::host::Log;
use w2r_core::host::Services;
use w2r_core::json::{Number, Value};
use w2r_core::loader::sha256_of;
use w2r_core::manifest::Build;
use w2r_core::plugins::load_all;

#[derive(Default)]
struct Lines(RefCell<Vec<String>>);

impl Log for Lines {
    fn line(&self, level: u32, plugin: &str, msg: &str) {
        self.0
            .borrow_mut()
            .push(format!("[{level}] {plugin}: {msg}"));
    }
}

impl Lines {
    fn has(&self, needle: &str) -> bool {
        self.0.borrow().iter().any(|l| l.contains(needle))
    }
    fn dump(&self) -> String {
        self.0.borrow().join("\n")
    }
}

fn fixtures() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        let target_dir = root.join("target").join("plugins");
        let status = Command::new(env!("CARGO"))
            .args(["build", "-q", "-p", "w2r-test-plugins", "--examples"])
            .args(["--target", env!("W2R_TARGET")])
            .env("CARGO_TARGET_DIR", &target_dir)
            .current_dir(&root)
            .status()
            .expect("run cargo");
        assert!(status.success(), "building the fixture plugins failed");
        target_dir
            .join(env!("W2R_TARGET"))
            .join("debug")
            .join("examples")
    })
}

fn s(v: &str) -> Value {
    Value::String(v.to_owned())
}

fn n(v: u64) -> Value {
    Value::Number(Number {
        text: v.to_string(),
    })
}

fn arr(items: Vec<Value>) -> Value {
    Value::Array(items)
}

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

fn set(v: &mut Value, key: &str, new: Value) {
    if let Value::Object(pairs) = v {
        match pairs.iter_mut().find(|(k, _)| k == key) {
            Some((_, old)) => *old = new,
            None => pairs.push((key.to_owned(), new)),
        }
    }
}

fn manifest(id: &str, entry: &str) -> Value {
    obj(vec![
        ("schema", n(1)),
        ("id", s(id)),
        ("name", s("Fixture")),
        ("version", s("1.0.0")),
        ("author", s("tests")),
        ("description", s("A fixture.")),
        ("entry", s(entry)),
        ("abi", n(1)),
        ("game_builds", arr(vec![s("1.0.2.2818")])),
        ("capabilities", arr(vec![])),
        ("affects_simulation", Value::Bool(false)),
        ("dependencies", arr(vec![])),
    ])
}

fn dep(id: &str, req: &str) -> Value {
    obj(vec![("id", s(id)), ("version_req", s(req))])
}

/// One plugin folder plus its state.json entry.
struct Plugin {
    id: &'static str,
    fixture: &'static str,
    /// `None` leaves plugin.json out; `Some(bytes)` writes those bytes verbatim.
    manifest: Option<Vec<u8>>,
    enabled: bool,
    approved: Vec<&'static str>,
    /// `Some` overrides the pin (otherwise the real hash).
    pin: Option<String>,
}

impl Plugin {
    fn new(id: &'static str, fixture: &'static str, manifest: Value) -> Plugin {
        Plugin {
            id,
            fixture,
            manifest: Some(manifest.to_json().into_bytes()),
            enabled: true,
            approved: Vec::new(),
            pin: None,
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Builds a host root under temp with the given plugins and returns it.
fn root_with(tag: &str, plugins: &[Plugin]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("w2r-load-all-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for sub in ["plugins", "config", "cache", "logs"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    let mut entries: Vec<(String, Value)> = Vec::new();
    for p in plugins {
        let folder = root.join("plugins").join(p.id);
        std::fs::create_dir_all(&folder).unwrap();
        let dll_name = format!("{}.dll", p.fixture);
        let dll = folder.join(&dll_name);
        std::fs::copy(fixtures().join(&dll_name), &dll).unwrap();
        if let Some(bytes) = &p.manifest {
            std::fs::write(folder.join("plugin.json"), bytes).unwrap();
        }
        let pin = p
            .pin
            .clone()
            .unwrap_or_else(|| hex(&sha256_of(&dll).unwrap()));
        let approved = p.approved.iter().map(|c| s(c)).collect();
        entries.push((
            p.id.to_owned(),
            obj(vec![
                ("version", s("1.0.0")),
                ("enabled", Value::Bool(p.enabled)),
                ("sha256", s(&pin)),
                ("approved_capabilities", arr(approved)),
            ]),
        ));
    }
    let state = obj(vec![("schema", n(1)), ("plugins", Value::Object(entries))]);
    std::fs::write(root.join("config").join("state.json"), state.to_json()).unwrap();
    root
}

fn running() -> Build {
    Build::parse("1.0.2.2818").unwrap()
}

#[test]
fn an_enabled_valid_plugin_loads_with_the_capabilities_it_declares() {
    let mut m = manifest("basic-plugin", "basic_plugin.dll");
    set(&mut m, "capabilities", arr(vec![s("chat.write")]));
    let mut basic = Plugin::new("basic-plugin", "basic_plugin", m);
    // A stale approval from an older version must not widen the plugin: it gets what its
    // manifest declares, which check 6 proved is within the approvals.
    basic.approved = vec!["chat.write", "game.read"];
    let off = Plugin {
        enabled: false,
        ..Plugin::new(
            "off-plugin",
            "crash_plugin",
            manifest("off-plugin", "crash_plugin.dll"),
        )
    };
    let root = root_with("ok", &[basic, off]);

    let log = Rc::new(Lines::default());
    let reg = load_all(&root, running(), log.clone(), None, Services::none()).registry;
    assert_eq!(reg.len(), 1, "{}", log.dump());
    let slot = reg.find("basic-plugin").unwrap();
    assert!(slot.is_active());
    assert!(slot.has_cap("chat.write"));
    assert!(!slot.has_cap("game.read"), "approved but not declared");
    assert!(!slot.has_cap("hooks.raw"));
    assert!(
        log.has("[1] basic-plugin: basic from basic_plugin"),
        "{}",
        log.dump()
    );
    assert!(log.has("1 of 1 enabled plugins loaded"), "{}", log.dump());
    assert!(reg.find("off-plugin").is_none());
    assert!(
        log.has("simulation fingerprint 00000000 (no simulation-affecting plugins)"),
        "{}",
        log.dump()
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// docs/load-policy.md, "Simulation fingerprint": loaded plugins with `affects_simulation` or
/// `hooks.raw` count, in id order, as (id, version, DLL hash); nothing else does.
#[test]
fn the_fingerprint_covers_simulation_and_raw_hook_plugins_only() {
    let mut sim = manifest("sim-plugin", "basic_plugin.dll");
    set(&mut sim, "affects_simulation", Value::Bool(true));
    set(&mut sim, "version", s("2.1.0"));
    let mut hooks = manifest("hooky", "basic_plugin.dll");
    set(&mut hooks, "capabilities", arr(vec![s("hooks.raw")]));
    let mut hooky = Plugin::new("hooky", "basic_plugin", hooks);
    hooky.approved = vec!["hooks.raw"];
    let plain = Plugin::new(
        "plain",
        "basic_plugin",
        manifest("plain", "basic_plugin.dll"),
    );
    let mut off = manifest("off-sim", "basic_plugin.dll");
    set(&mut off, "affects_simulation", Value::Bool(true));
    let off = Plugin {
        enabled: false,
        ..Plugin::new("off-sim", "basic_plugin", off)
    };
    // Refused before loading (bad pin): not part of the session, so not in the fingerprint.
    let mut refused_m = manifest("refused", "basic_plugin.dll");
    set(&mut refused_m, "affects_simulation", Value::Bool(true));
    let refused = Plugin {
        pin: Some("00".repeat(32)),
        ..Plugin::new("refused", "basic_plugin", refused_m)
    };
    let root = root_with(
        "fingerprint",
        &[
            Plugin::new("sim-plugin", "basic_plugin", sim),
            hooky,
            plain,
            off,
            refused,
        ],
    );

    let log = Rc::new(Lines::default());
    let loaded = load_all(&root, running(), log.clone(), None, Services::none());
    assert_eq!(
        loaded.simulation,
        vec!["hooky".to_owned(), "sim-plugin".to_owned()]
    );
    let dll = root
        .join("plugins")
        .join("sim-plugin")
        .join("basic_plugin.dll");
    let sha = sha256_of(&dll).unwrap();
    let expected = fingerprint::compute(&[
        fingerprint::Entry {
            id: "sim-plugin",
            version: "2.1.0",
            sha256: &sha,
        },
        fingerprint::Entry {
            id: "hooky",
            version: "1.0.0",
            sha256: &sha,
        },
    ])
    .unwrap();
    assert_eq!(loaded.fingerprint, expected);
    assert_ne!(loaded.fingerprint, [0; 4]);
    assert!(
        log.has(&format!(
            "simulation fingerprint {} (hooky, sim-plugin)",
            fingerprint::hex(&expected)
        )),
        "{}",
        log.dump()
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn each_check_refuses_with_its_code_and_leaves_no_slot() {
    let mut old_build = manifest("old-build", "basic_plugin.dll");
    set(
        &mut old_build,
        "game_builds",
        arr(vec![s("1.0.0.1-1.0.2.2817")]),
    );
    let mut new_abi = manifest("new-abi", "basic_plugin.dll");
    set(&mut new_abi, "abi", n(2));
    let mut needs_caps = manifest("needs-caps", "basic_plugin.dll");
    set(&mut needs_caps, "capabilities", arr(vec![s("hooks.raw")]));
    let mut dep_missing = manifest("dep-missing", "basic_plugin.dll");
    set(
        &mut dep_missing,
        "dependencies",
        arr(vec![dep("nope", ">=1.0.0")]),
    );

    let plugins = vec![
        Plugin {
            manifest: None,
            ..Plugin::new(
                "no-manifest",
                "basic_plugin",
                manifest("no-manifest", "basic_plugin.dll"),
            )
        },
        Plugin {
            manifest: Some(b"{ not json".to_vec()),
            ..Plugin::new("bad-manifest", "basic_plugin", manifest("x", "x.dll"))
        },
        Plugin::new(
            "wrong-id",
            "basic_plugin",
            manifest("other-id", "basic_plugin.dll"),
        ),
        Plugin::new("old-build", "basic_plugin", old_build),
        Plugin::new("new-abi", "basic_plugin", new_abi),
        Plugin {
            pin: Some("00".repeat(32)),
            ..Plugin::new(
                "bad-pin",
                "basic_plugin",
                manifest("bad-pin", "basic_plugin.dll"),
            )
        },
        Plugin::new("needs-caps", "basic_plugin", needs_caps),
        Plugin::new("dep-missing", "basic_plugin", dep_missing),
        Plugin::new(
            "refuses",
            "load_fails",
            manifest("refuses", "load_fails.dll"),
        ),
        Plugin::new(
            "no-entry",
            "null_entry",
            manifest("no-entry", "null_entry.dll"),
        ),
    ];
    let root = root_with("refusals", &plugins);
    let log = Rc::new(Lines::default());
    let reg = load_all(&root, running(), log.clone(), None, Services::none()).registry;

    for (id, code) in [
        ("no-manifest", "E_STATE: plugin.json is missing"),
        ("bad-manifest", "E_MANIFEST: not_json"),
        ("wrong-id", "E_STATE: manifest id other-id differs"),
        ("old-build", "E_BUILD: game build 1.0.2.2818"),
        ("new-abi", "E_ABI: needs ABI 2"),
        ("bad-pin", "E_HASH: sha256 of basic_plugin.dll differs"),
        ("needs-caps", "E_CAPS: capability hooks.raw"),
        ("dep-missing", "E_DEP: dependency nope is not loaded"),
        ("refuses", "disabled: E_LOAD: on_load returned -42"),
        (
            "no-entry",
            "disabled: E_ENTRY: w2r_plugin_main returned null",
        ),
    ] {
        assert!(
            log.has(&format!("{id}: {code}")),
            "{id}: expected {code:?} in\n{}",
            log.dump()
        );
    }
    // Refused before check 9: no slot. Refused at 9 or 10: a disabled slot.
    assert_eq!(reg.len(), 2, "{}", log.dump());
    assert!(!reg.find("refuses").unwrap().is_active());
    assert!(!reg.find("no-entry").unwrap().is_active());
    assert!(log.has("0 of 10 enabled plugins loaded"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn dependencies_load_first_and_version_and_cycles_are_enforced() {
    let mut needs_basic = manifest("needs-basic", "basic_plugin.dll");
    set(
        &mut needs_basic,
        "dependencies",
        arr(vec![dep("basic-plugin", ">=1.0.0,<2.0.0")]),
    );
    let mut wants_newer = manifest("wants-newer", "basic_plugin.dll");
    set(
        &mut wants_newer,
        "dependencies",
        arr(vec![dep("basic-plugin", ">=9.0.0")]),
    );
    let mut cycle_a = manifest("cycle-a", "basic_plugin.dll");
    set(
        &mut cycle_a,
        "dependencies",
        arr(vec![dep("cycle-b", ">=1.0.0")]),
    );
    let mut cycle_b = manifest("cycle-b", "basic_plugin.dll");
    set(
        &mut cycle_b,
        "dependencies",
        arr(vec![dep("cycle-a", ">=1.0.0")]),
    );
    let mut after_crasher = manifest("after-crasher", "basic_plugin.dll");
    set(
        &mut after_crasher,
        "dependencies",
        arr(vec![dep("crasher", ">=1.0.0")]),
    );

    // Listed so that id order alone would load dependents first.
    let plugins = vec![
        Plugin::new("after-crasher", "basic_plugin", after_crasher),
        Plugin::new("cycle-a", "basic_plugin", cycle_a),
        Plugin::new("cycle-b", "basic_plugin", cycle_b),
        Plugin::new(
            "crasher",
            "load_fails",
            manifest("crasher", "load_fails.dll"),
        ),
        Plugin::new(
            "basic-plugin",
            "basic_plugin",
            manifest("basic-plugin", "basic_plugin.dll"),
        ),
        Plugin::new("needs-basic", "basic_plugin", needs_basic),
        Plugin::new("wants-newer", "basic_plugin", wants_newer),
    ];
    let root = root_with("deps", &plugins);
    let log = Rc::new(Lines::default());
    let reg = load_all(&root, running(), log.clone(), None, Services::none()).registry;

    let basic = reg.find("basic-plugin").expect("basic loaded");
    let needs = reg.find("needs-basic").expect("dependent loaded");
    assert!(basic.is_active() && needs.is_active(), "{}", log.dump());
    assert!(basic.index < needs.index, "dependency loads first");
    assert!(reg.find("wants-newer").is_none());
    assert!(
        log.has("wants-newer: E_DEP: dependency basic-plugin"),
        "{}",
        log.dump()
    );
    assert!(log.has("cycle-a: E_DEP: part of a dependency cycle"));
    assert!(log.has("cycle-b: E_DEP: part of a dependency cycle"));
    // crasher reaches check 10 and fails, so after-crasher fails check 8 at its turn.
    assert!(!reg.find("crasher").unwrap().is_active());
    assert!(reg.find("after-crasher").is_none());
    assert!(
        log.has("after-crasher: E_DEP: dependency crasher is not loaded"),
        "{}",
        log.dump()
    );
    assert!(log.has("2 of 7 enabled plugins loaded"), "{}", log.dump());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_missing_or_bad_state_file_loads_nothing() {
    let root = std::env::temp_dir().join(format!("w2r-load-all-{}-nostate", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("config")).unwrap();
    let log = Rc::new(Lines::default());
    let reg = load_all(&root, running(), log.clone(), None, Services::none()).registry;
    assert!(reg.is_empty());
    assert!(log.has("no config\\state.json"));

    std::fs::write(
        root.join("config").join("state.json"),
        b"{\"schema\": 2, \"plugins\": {}}",
    )
    .unwrap();
    let log = Rc::new(Lines::default());
    let reg = load_all(&root, running(), log.clone(), None, Services::none()).registry;
    assert!(reg.is_empty());
    assert!(log.has("state.json refused (bad_schema)"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_loaded_replacer_refuses_the_plugin_it_replaces_only_within_its_version_range() {
    let replaces = |id: &str, entry: &str, id_of_old: &str, req: &str| {
        let mut m = manifest(id, entry);
        set(&mut m, "replaces", arr(vec![dep(id_of_old, req)]));
        m
    };
    // "a-old" sorts before "z-new", so id order alone would load it first.
    let plugins = vec![
        Plugin::new("a-old", "basic_plugin", manifest("a-old", "basic_plugin.dll")),
        Plugin::new(
            "z-new",
            "basic_plugin",
            replaces("z-new", "basic_plugin.dll", "a-old", "<2.0.0"),
        ),
        // Replaces a version this plugin is not (1.0.0 is below >=5.0.0): both load.
        Plugin::new("b-kept", "basic_plugin", manifest("b-kept", "basic_plugin.dll")),
        Plugin::new(
            "y-other",
            "basic_plugin",
            replaces("y-other", "basic_plugin.dll", "b-kept", ">=5.0.0"),
        ),
        // The replacer fails to load, so the plugin it replaces keeps running.
        Plugin::new("c-fallback", "basic_plugin", manifest("c-fallback", "basic_plugin.dll")),
        Plugin::new(
            "x-broken",
            "load_fails",
            replaces("x-broken", "load_fails.dll", "c-fallback", "<2.0.0"),
        ),
    ];
    let root = root_with("replaces", &plugins);
    let log = Rc::new(Lines::default());
    let reg = load_all(&root, running(), log.clone(), None, Services::none()).registry;

    assert!(reg.find("z-new").unwrap().is_active(), "{}", log.dump());
    assert!(reg.find("a-old").is_none(), "{}", log.dump());
    assert!(
        log.has("a-old: E_REPLACED: replaced by z-new"),
        "{}",
        log.dump()
    );
    assert!(reg.find("b-kept").unwrap().is_active(), "{}", log.dump());
    assert!(reg.find("y-other").unwrap().is_active());
    assert!(!reg.find("x-broken").unwrap().is_active());
    assert!(reg.find("c-fallback").unwrap().is_active(), "{}", log.dump());
    assert!(!log.has("c-fallback: E_REPLACED"));
    let _ = std::fs::remove_dir_all(&root);
}
