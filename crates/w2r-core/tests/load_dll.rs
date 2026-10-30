//! End-to-end: real plugin DLLs (crates/test-plugins examples) through the pinned loader, the
//! registry and the crash guard, in a process of the same bitness as the test target.

#![cfg(windows)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::OnceLock;

use w2r_abi::*;
use w2r_core::dispatch::{LoadError, Registry};
use w2r_core::host::{Log, State};
use w2r_core::loader::sha256_of;

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
    fn count(&self, needle: &str) -> usize {
        self.0
            .borrow()
            .iter()
            .filter(|l| l.contains(needle))
            .count()
    }
}

/// Builds the fixture DLLs once, for this test's own target, in a separate target dir so the
/// nested cargo does not contend with the outer one.
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

fn dll(name: &str) -> PathBuf {
    fixtures().join(format!("{name}.dll"))
}

fn registry() -> (Registry, Rc<Lines>) {
    let log = Rc::new(Lines::default());
    (Registry::new(log.clone()), log)
}

fn load(reg: &mut Registry, id: &str, path: &Path) -> Result<usize, LoadError> {
    let pin = sha256_of(path).expect("hash fixture");
    reg.load_dll(id, path, &pin, &[], Vec::new())
}

#[test]
fn basic_plugin_logs_on_load_and_receives_its_events() {
    let (mut reg, log) = registry();
    load(&mut reg, "basic", &dll("basic_plugin")).unwrap();
    assert!(log.has("[1] basic: basic from basic_plugin"));

    reg.dispatch(W2R_EVT_LOBBY_LEAVE, core::ptr::null());
    assert_eq!(log.count("event 1"), 0);
    assert_eq!(log.count("[1] basic: event 2"), 1);
    reg.dispatch(W2R_EVT_LOBBY_ENTER, core::ptr::null());
    assert!(log.has("[1] basic: event 1"));
    assert!(reg.find("basic").unwrap().is_active());
}

#[test]
fn a_crashing_dll_is_disabled_and_the_game_and_other_plugins_go_on() {
    let (mut reg, log) = registry();
    // The crasher comes first so dispatch has to carry on past it.
    load(&mut reg, "crash", &dll("crash_plugin")).unwrap();
    load(&mut reg, "basic", &dll("basic_plugin")).unwrap();

    reg.dispatch(W2R_EVT_LOBBY_ENTER, core::ptr::null());
    reg.dispatch(W2R_EVT_LOBBY_ENTER, core::ptr::null());

    let State::Disabled { reason } = reg.find("crash").unwrap().state() else {
        panic!("crash plugin is still active");
    };
    assert!(
        reason.starts_with("on_event(1): access violation"),
        "{reason}"
    );
    assert_eq!(
        log.count("about to crash"),
        1,
        "disabled plugin was called again"
    );
    assert!(!log.has("unreachable"));
    assert_eq!(log.count("[1] basic: event 1"), 2);
    assert!(reg.find("basic").unwrap().is_active());
}

#[test]
fn entry_and_load_failures_are_refused_with_the_spec_codes() {
    let (mut reg, log) = registry();

    let err = load(&mut reg, "null", &dll("null_entry")).unwrap_err();
    assert_eq!(err.code, "E_ENTRY");
    assert_eq!(err.detail, "w2r_plugin_main returned null");

    let err = load(&mut reg, "refuses", &dll("load_fails")).unwrap_err();
    assert_eq!(err.code, "E_LOAD");
    assert_eq!(err.detail, "on_load returned -42");
    assert!(log.has("[2] refuses: refusing to load"));

    // A real DLL without the export. System32 is redirected to SysWOW64 in a 32-bit process.
    let system32 = PathBuf::from(std::env::var("SystemRoot").unwrap()).join("System32");
    let kernel32 = system32.join("kernel32.dll");
    let err = load(&mut reg, "k32", &kernel32).unwrap_err();
    assert_eq!(err.code, "E_ENTRY");
    assert_eq!(err.detail, "no w2r_plugin_main export");

    let missing = fixtures().join("does_not_exist.dll");
    let err = reg
        .load_dll("missing", &missing, &[0u8; 32], &[], Vec::new())
        .unwrap_err();
    assert_eq!(err.code, "E_ENTRY");
    assert!(err.detail.starts_with("cannot open"), "{err}");

    let err = reg
        .load_dll("badpin", &dll("basic_plugin"), &[0u8; 32], &[], Vec::new())
        .unwrap_err();
    assert_eq!(err.code, "E_HASH");
    assert!(err.detail.contains("differs from pin 0000"), "{err}");

    // Refusals before the entry point leave no slot; refusals at the entry point leave a
    // disabled one.
    assert_eq!(reg.len(), 2);
    assert!(matches!(
        reg.find("null").unwrap().state(),
        State::Disabled { .. }
    ));
    assert!(matches!(
        reg.find("refuses").unwrap().state(),
        State::Disabled { .. }
    ));
    assert!(log.has("[3] k32: E_ENTRY: no w2r_plugin_main export"));
    assert!(log.has("[3] badpin: E_HASH: sha256"));
}
