//! The widget API (`W2rUi`, docs/abi.md) and the plugin manager's list. Widgets are behind
//! [`Widgets`] so the plugin-facing rules are testable with a fake; `options.rs` binds them to
//! the game's own menu widgets.
//!
//! Each `W2rUi` call draws one row. Strings are at most 128 bytes of UTF-8; a bad string or a
//! null value pointer makes the call a no-op that reports "not pressed / not changed".

use core::ffi::c_void;
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use w2r_abi::{W2rStr, W2rUi};

pub const MAX_TEXT: usize = 128;
/// Row height in the game's UI space (2160 high), the options screen's value.
pub const ROW_H: f32 = 90.0;
/// The gap the plugin screen puts between rows (set on the game's layout context).
pub const ROW_GAP: f32 = 24.0;
/// What a plugin's card needs beyond its rows: the frame's own padding, top and bottom.
pub const CARD_PAD: f32 = 20.0;
/// The empty row at the top of a card, under the frame's border.
pub const CARD_TOP_H: f32 = 70.0;
/// An edit row. The game's edit box pads its text, and the plugin screen's font is 80 high, so a
/// 90-high row clips the text away and leaves only the caret (seen in game).
pub const EDIT_ROW_H: f32 = 150.0;
/// Room added to a measured text width, so neighbouring cells do not touch.
pub const CELL_PAD: f32 = 24.0;
/// The author's name on a plugin card: the game's gold, packed as Nuklear packs colors.
pub const AUTHOR_COLOUR: u32 = 0xff00_ccff;

/// One row of widgets. `label` and friends each take the whole row.
pub trait Widgets {
    fn row(&self, height: f32, cols: i32);
    fn label(&self, text: &str);
    /// True when pressed this frame.
    fn button(&self, text: &str) -> bool;
    /// True when the value changed this frame.
    fn checkbox(&self, text: &str, value: &mut bool) -> bool;
    /// True when the value changed this frame. `min < max`.
    fn slider(&self, text: &str, value: &mut i32, min: i32, max: i32) -> bool;
    /// An edit box over `buf` (NUL-terminated, edited in place); true when the text changed.
    fn text_edit(&self, _text: &str, _buf: &mut [u8]) -> bool {
        false
    }
    /// Greys out everything until `disable_end`.
    fn disable_begin(&self) {}
    fn disable_end(&self) {}
    /// Starts a framed card `height` tall (its own row) for the plugin at `key`; false when it
    /// is not shown (scrolled out of view), in which case nothing is drawn or ended.
    fn card_begin(&self, _key: usize, _height: f32) -> bool {
        true
    }
    fn card_end(&self) {}
    /// How wide `text` is drawn, in UI units.
    fn text_width(&self, text: &str) -> f32 {
        text.len() as f32 * 40.0
    }
    /// How many lines `text` wraps to at `width`.
    fn text_lines(&self, _text: &str, _width: f32) -> i32 {
        1
    }
    /// The height of one line of text.
    fn line_height(&self) -> f32 {
        ROW_H
    }
    /// The width a row's cells share.
    fn row_width(&self) -> f32 {
        1000.0
    }
    /// A row of cells of the given widths, left to right. A width of 0 shares what the others
    /// leave.
    fn row_cells(&self, height: f32, widths: &[f32]) {
        self.row(height, widths.len() as i32);
    }
    fn label_coloured(&self, text: &str, _colour: u32) {
        self.label(text);
    }
    /// A label against the right edge of its cell.
    fn label_right(&self, text: &str) {
        self.label(text);
    }
    /// A label that wraps onto as many lines as its row is tall.
    fn label_wrapped(&self, text: &str) {
        self.label(text);
    }
}

/// Forwards to `inner` and adds up the height of the rows drawn through it, so a card can be
/// sized to its content (the frame needs its height before the content is drawn).
struct Measured<'a> {
    inner: &'a dyn Widgets,
    height: Cell<f32>,
}

impl Widgets for Measured<'_> {
    fn row(&self, height: f32, cols: i32) {
        self.height.set(self.height.get() + height + ROW_GAP);
        self.inner.row(height, cols);
    }
    fn label(&self, text: &str) {
        self.inner.label(text);
    }
    fn button(&self, text: &str) -> bool {
        self.inner.button(text)
    }
    fn checkbox(&self, text: &str, value: &mut bool) -> bool {
        self.inner.checkbox(text, value)
    }
    fn slider(&self, text: &str, value: &mut i32, min: i32, max: i32) -> bool {
        self.inner.slider(text, value, min, max)
    }
    fn text_edit(&self, text: &str, buf: &mut [u8]) -> bool {
        self.inner.text_edit(text, buf)
    }
    fn disable_begin(&self) {
        self.inner.disable_begin();
    }
    fn disable_end(&self) {
        self.inner.disable_end();
    }
    fn text_width(&self, text: &str) -> f32 {
        self.inner.text_width(text)
    }
    fn text_lines(&self, text: &str, width: f32) -> i32 {
        self.inner.text_lines(text, width)
    }
    fn line_height(&self) -> f32 {
        self.inner.line_height()
    }
    fn row_width(&self) -> f32 {
        self.inner.row_width()
    }
    fn row_cells(&self, height: f32, widths: &[f32]) {
        self.height.set(self.height.get() + height + ROW_GAP);
        self.inner.row_cells(height, widths);
    }
    fn label_coloured(&self, text: &str, colour: u32) {
        self.inner.label_coloured(text, colour);
    }
    fn label_right(&self, text: &str) {
        self.inner.label_right(text);
    }
    fn label_wrapped(&self, text: &str) {
        self.inner.label_wrapped(text);
    }
}

thread_local! {
    /// Each plugin card's height as last measured, by plugin id (the first frame guesses).
    static CARD_HEIGHTS: std::cell::RefCell<std::collections::HashMap<String, f32>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// What a plugin's `on_ui` call sees behind `W2rUi.ctx`.
struct Frame<'a> {
    widgets: &'a dyn Widgets,
    /// `set_active(0)` is in effect; closed when the frame ends if the plugin forgot.
    dimmed: Cell<bool>,
}

/// The longest edit buffer a plugin may hand over.
pub const MAX_EDIT: usize = 256;

thread_local! {
    /// The one frame whose table may be used right now (its address), or 0. A `W2rUi` kept
    /// past `on_ui`, or used from another thread, no longer matches and every call is a no-op
    /// instead of a read through a dead pointer.
    static ACTIVE: Cell<usize> = const { Cell::new(0) };
}

/// Runs `f` with a table over `widgets`. The table is live only inside `f`; the frame behind
/// it never moves, and the previous active frame (if `f` nests) is restored on the way out.
pub fn with_frame<R>(widgets: &dyn Widgets, f: impl FnOnce(&W2rUi) -> R) -> R {
    struct Restore(usize);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|a| a.set(self.0));
        }
    }
    let mut frame = Frame {
        widgets,
        dimmed: Cell::new(false),
    };
    let ctx: *mut Frame<'_> = &mut frame;
    let _restore = Restore(ACTIVE.with(|a| a.replace(ctx as usize)));
    let table = W2rUi {
        struct_size: size_of::<W2rUi>() as u32,
        ctx: ctx.cast(),
        label: cb::label,
        button: cb::button,
        checkbox: cb::checkbox,
        slider_int: cb::slider_int,
        text_edit: cb::text_edit,
        set_active: cb::set_active,
    };
    let result = f(&table);
    if frame.dimmed.get() {
        widgets.disable_end();
    }
    result
}

mod cb {
    use super::*;

    /// Only the frame that is live on this thread; anything else is refused.
    unsafe fn frame<'a>(ctx: *mut c_void) -> Option<&'a Frame<'a>> {
        if ctx.is_null() || ACTIVE.with(|a| a.get()) != ctx as usize {
            return None;
        }
        unsafe { ctx.cast::<Frame<'a>>().as_ref() }
    }

    /// Host code must not unwind into the plugin.
    fn run(f: impl FnOnce() -> i32) -> i32 {
        catch_unwind(AssertUnwindSafe(f)).unwrap_or(0)
    }

    unsafe fn text<'a>(s: W2rStr) -> Option<&'a str> {
        if s.len as usize > MAX_TEXT || (s.ptr.is_null() && s.len != 0) {
            return None;
        }
        if s.len == 0 {
            return Some("");
        }
        let bytes = unsafe { core::slice::from_raw_parts(s.ptr.cast::<u8>(), s.len as usize) };
        core::str::from_utf8(bytes).ok()
    }

    pub unsafe extern "C" fn label(ctx: *mut c_void, text: W2rStr) {
        run(|| {
            let (Some(f), Some(t)) = (unsafe { frame(ctx) }, unsafe { self::text(text) }) else {
                return 0;
            };
            f.widgets.row(ROW_H, 1);
            f.widgets.label(t);
            0
        });
    }

    pub unsafe extern "C" fn button(ctx: *mut c_void, text: W2rStr) -> i32 {
        run(|| {
            let (Some(f), Some(t)) = (unsafe { frame(ctx) }, unsafe { self::text(text) }) else {
                return 0;
            };
            f.widgets.row(ROW_H, 1);
            f.widgets.button(t) as i32
        })
    }

    pub unsafe extern "C" fn checkbox(ctx: *mut c_void, text: W2rStr, value: *mut i32) -> i32 {
        run(|| {
            let (Some(f), Some(t)) = (unsafe { frame(ctx) }, unsafe { self::text(text) }) else {
                return 0;
            };
            if value.is_null() {
                return 0;
            }
            let mut v = unsafe { *value } != 0;
            f.widgets.row(ROW_H, 1);
            if f.widgets.checkbox(t, &mut v) {
                unsafe { *value = v as i32 };
                1
            } else {
                0
            }
        })
    }

    pub unsafe extern "C" fn slider_int(
        ctx: *mut c_void,
        text: W2rStr,
        value: *mut i32,
        min: i32,
        max: i32,
    ) -> i32 {
        run(|| {
            let (Some(f), Some(t)) = (unsafe { frame(ctx) }, unsafe { self::text(text) }) else {
                return 0;
            };
            if value.is_null() || min >= max {
                return 0;
            }
            let mut v = unsafe { *value }.clamp(min, max);
            f.widgets.row(ROW_H, 2);
            if f.widgets.slider(t, &mut v, min, max) {
                unsafe { *value = v };
                1
            } else {
                0
            }
        })
    }

    pub unsafe extern "C" fn text_edit(
        ctx: *mut c_void,
        text: W2rStr,
        buf: *mut u8,
        cap: u32,
    ) -> i32 {
        run(|| {
            let (Some(f), Some(t)) = (unsafe { frame(ctx) }, unsafe { self::text(text) }) else {
                return 0;
            };
            if buf.is_null() || cap == 0 || cap as usize > MAX_EDIT {
                return 0;
            }
            let bytes = unsafe { core::slice::from_raw_parts_mut(buf, cap as usize) };
            // The plugin's text must end inside its buffer; make it so before the game sees it.
            if !bytes.contains(&0) {
                bytes[cap as usize - 1] = 0;
            }
            f.widgets.row(EDIT_ROW_H, 2);
            f.widgets.text_edit(t, bytes) as i32
        })
    }

    pub unsafe extern "C" fn set_active(ctx: *mut c_void, active: i32) {
        run(|| {
            let Some(f) = (unsafe { frame(ctx) }) else {
                return 0;
            };
            match (active != 0, f.dimmed.get()) {
                (false, false) => {
                    f.widgets.disable_begin();
                    f.dimmed.set(true);
                }
                (true, true) => {
                    f.widgets.disable_end();
                    f.dimmed.set(false);
                }
                _ => {}
            }
            0
        });
    }
}

/// A plugin as the manager shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRow {
    pub id: String,
    /// The name from `plugin.json`, or the id when it cannot be read.
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub description: Option<String>,
    /// `enabled` in state.json, which takes effect at the next launch.
    pub enabled: bool,
    /// `enabled` as it was at launch.
    pub enabled_at_launch: bool,
    /// Whether the plugin is running this session.
    pub running: bool,
    /// Its callbacks have been slow (`Slot::is_slow`).
    pub slow: bool,
    /// Why it is not running, when that is a failure: a fault, or a refusal at load.
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// The user flipped a plugin's enabled flag.
    Toggle(usize),
}

/// A card's status line. A restart is needed when the enable box changed since launch.
pub fn status_text(row: &PluginRow) -> String {
    let state = if row.running { "Running" } else { "Disabled" };
    let mut notes = Vec::new();
    if row.running && row.slow {
        notes.push("slow callbacks");
    }
    if row.error.is_some() {
        notes.push("error");
    }
    if row.enabled != row.enabled_at_launch {
        notes.push("restart to update");
    }
    if notes.is_empty() {
        format!("Status: {state}")
    } else {
        format!("Status: {state} ({})", notes.join(", "))
    }
}

/// `text` wrapped onto as many lines as it needs, in a row of its own.
fn paragraph(w: &dyn Widgets, text: &str) {
    let lines = w.text_lines(text, w.row_width()).max(1);
    let height = (lines as f32 * w.line_height()).max(ROW_H);
    w.row_cells(height, &[0.0]);
    w.label_wrapped(text);
}

/// The plugin manager's list (docs/settings-screen.md): the session's simulation fingerprint,
/// then a card per plugin with its name, author and version, its status and enable box, its
/// description, and under them any rows the plugin still draws itself through `on_ui`.
/// `draw_section` draws a running plugin's own rows.
pub fn draw_plugin_list(
    w: &dyn Widgets,
    fingerprint: [u8; 4],
    simulation: &[String],
    rows: &mut [PluginRow],
    mut draw_section: impl FnMut(&dyn Widgets, usize),
) -> Action {
    let mut action = Action::None;
    w.row(ROW_H, 1);
    w.label(&format!(
        "Fingerprint: {} ({})",
        crate::fingerprint::hex(&fingerprint),
        if simulation.is_empty() {
            "no plugins".to_owned()
        } else {
            simulation.join(", ")
        }
    ));
    if rows.is_empty() {
        w.row(ROW_H, 1);
        w.label("No plugins installed. Use the W2R Mod Loader to add some.");
    }
    for (i, row) in rows.iter_mut().enumerate() {
        // Each plugin sits in its own framed card, sized from the last frame's measurement.
        let guess = 6.0 * (ROW_H + ROW_GAP) + CARD_PAD;
        let height = CARD_HEIGHTS.with(|h| h.borrow().get(&row.id).copied().unwrap_or(guess));
        if !w.card_begin(i, height) {
            continue;
        }
        let m = Measured {
            inner: w,
            height: Cell::new(CARD_TOP_H + ROW_GAP),
        };

        // The name, with the version against the right edge, then the author in gold.
        let version_w = m.text_width(&row.version) + CELL_PAD;
        m.row_cells(ROW_H, &[0.0, version_w]);
        m.label(&row.name);
        m.label_right(&row.version);
        if let Some(author) = &row.author {
            let caption_w = m.text_width("Author:") + CELL_PAD;
            m.row_cells(ROW_H, &[caption_w, 0.0]);
            m.label("Author:");
            m.label_coloured(author, AUTHOR_COLOUR);
        }

        // The status, with the enable box against the right edge.
        let enable_w = m.text_width(" Enable") + ROW_H + CELL_PAD;
        m.row_cells(ROW_H, &[0.0, enable_w]);
        m.label(&status_text(row));
        let mut enabled = row.enabled;
        if m.checkbox("Enable", &mut enabled) {
            row.enabled = enabled;
            action = Action::Toggle(i);
        }

        if let Some(error) = &row.error {
            paragraph(&m, &format!("Error: {error}"));
        }
        if let Some(description) = row.description.as_deref().filter(|d| !d.is_empty()) {
            paragraph(&m, &format!("Description: {description}"));
        }

        if row.running {
            draw_section(&m, i);
        }
        w.card_end();
        let needed = m.height.get() + CARD_PAD;
        CARD_HEIGHTS.with(|h| h.borrow_mut().insert(row.id.clone(), needed));
    }
    action
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// Records every call; `press` / `flip` / `set` script what the user does.
    #[derive(Default)]
    pub struct FakeWidgets {
        pub calls: RefCell<Vec<String>>,
        pub press: RefCell<Vec<String>>,
        pub flip: RefCell<Vec<String>>,
        pub set: RefCell<BTreeMap<String, i32>>,
        /// Text the user types into the edit box with this label.
        pub typed: RefCell<BTreeMap<String, String>>,
    }

    impl Widgets for FakeWidgets {
        fn row(&self, height: f32, cols: i32) {
            self.calls.borrow_mut().push(format!("row {height} {cols}"));
        }
        fn label(&self, text: &str) {
            self.calls.borrow_mut().push(format!("label {text:?}"));
        }
        fn button(&self, text: &str) -> bool {
            self.calls.borrow_mut().push(format!("button {text:?}"));
            self.press.borrow().iter().any(|p| p == text)
        }
        fn checkbox(&self, text: &str, value: &mut bool) -> bool {
            self.calls
                .borrow_mut()
                .push(format!("checkbox {text:?} {value}"));
            if self.flip.borrow().iter().any(|p| p == text) {
                *value = !*value;
                true
            } else {
                false
            }
        }
        fn text_edit(&self, text: &str, buf: &mut [u8]) -> bool {
            self.calls.borrow_mut().push(format!("text_edit {text:?}"));
            let Some(new) = self.typed.borrow().get(text).cloned() else {
                return false;
            };
            let n = new.len().min(buf.len() - 1);
            buf[..n].copy_from_slice(&new.as_bytes()[..n]);
            buf[n] = 0;
            true
        }
        fn disable_begin(&self) {
            self.calls.borrow_mut().push("disable_begin".into());
        }
        fn disable_end(&self) {
            self.calls.borrow_mut().push("disable_end".into());
        }
        fn slider(&self, text: &str, value: &mut i32, min: i32, max: i32) -> bool {
            self.calls
                .borrow_mut()
                .push(format!("slider {text:?} {value} {min} {max}"));
            match self.set.borrow().get(text) {
                Some(&v) if v != *value => {
                    *value = v;
                    true
                }
                _ => false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeWidgets;
    use super::*;

    fn s(text: &str) -> W2rStr {
        W2rStr {
            ptr: text.as_ptr().cast(),
            len: text.len() as u32,
        }
    }

    #[test]
    fn the_table_routes_each_widget_to_one_row() {
        let fake = FakeWidgets::default();
        fake.press.borrow_mut().push("Go".into());
        fake.flip.borrow_mut().push("Flag".into());
        fake.set.borrow_mut().insert("Vol".into(), 7);
        let mut flag = 0i32;
        let mut vol = 3i32;
        let mut untouched = 1i32;
        with_frame(&fake, |ui| unsafe {
            (ui.label)(ui.ctx, s("Hello"));
            assert_eq!((ui.button)(ui.ctx, s("Go")), 1);
            assert_eq!((ui.button)(ui.ctx, s("Stop")), 0);
            assert_eq!((ui.checkbox)(ui.ctx, s("Flag"), &mut flag), 1);
            assert_eq!(flag, 1);
            assert_eq!((ui.checkbox)(ui.ctx, s("Other"), &mut untouched), 0);
            assert_eq!(untouched, 1);
            assert_eq!((ui.slider_int)(ui.ctx, s("Vol"), &mut vol, 0, 10), 1);
            assert_eq!(vol, 7);
            assert_eq!(
                (ui.slider_int)(ui.ctx, s("Vol"), &mut vol, 0, 10),
                0,
                "already 7"
            );
        });
        let calls = fake.calls.borrow();
        assert_eq!(calls[0], "row 90 1");
        assert_eq!(calls[1], "label \"Hello\"");
        assert_eq!(calls[3], "button \"Go\"");
        assert_eq!(calls[7], "checkbox \"Flag\" false");
        assert_eq!(calls[11], "slider \"Vol\" 3 0 10");
    }

    #[test]
    fn bad_arguments_are_no_ops() {
        let fake = FakeWidgets::default();
        fake.press.borrow_mut().push("Go".into());
        let long = "x".repeat(MAX_TEXT + 1);
        let mut v = 5i32;
        with_frame(&fake, |ui| unsafe {
            (ui.label)(ui.ctx, s(&long));
            (ui.label)(
                ui.ctx,
                W2rStr {
                    ptr: core::ptr::null(),
                    len: 3,
                },
            );
            (ui.label)(
                ui.ctx,
                W2rStr {
                    ptr: [0xFFu8, 0xFE].as_ptr().cast(),
                    len: 2,
                },
            );
            assert_eq!((ui.button)(ui.ctx, s(&long)), 0);
            assert_eq!((ui.checkbox)(ui.ctx, s("x"), core::ptr::null_mut()), 0);
            assert_eq!((ui.slider_int)(ui.ctx, s("x"), &mut v, 5, 5), 0);
            assert_eq!(
                (ui.slider_int)(ui.ctx, s("x"), core::ptr::null_mut(), 0, 5),
                0
            );
            assert_eq!((ui.button)(core::ptr::null_mut(), s("Go")), 0);
            (ui.label)(ui.ctx, s(""));
        });
        assert_eq!(fake.calls.borrow().len(), 2, "{:?}", fake.calls.borrow());
    }

    #[test]
    fn a_table_kept_past_its_frame_is_inert() {
        let fake = FakeWidgets::default();
        fake.press.borrow_mut().push("Go".into());
        let kept = with_frame(&fake, |ui| {
            unsafe { assert_eq!((ui.button)(ui.ctx, s("Go")), 1) };
            *ui
        });
        unsafe { assert_eq!((kept.button)(kept.ctx, s("Go")), 0) };
        // A later frame does not revive the old table, and the old one cannot reach the new.
        with_frame(&fake, |ui| unsafe {
            assert_eq!((kept.button)(kept.ctx, s("Go")), 0);
            assert_eq!((ui.button)(ui.ctx, s("Go")), 1);
        });
        assert_eq!(fake.calls.borrow().len(), 4, "{:?}", fake.calls.borrow());
    }

    #[test]
    fn status_lines_say_when_a_restart_is_needed() {
        let row = |running, enabled, launched| PluginRow {
            id: "p".into(),
            name: "p".into(),
            version: "1.0.0".into(),
            author: None,
            description: None,
            enabled,
            enabled_at_launch: launched,
            running,
            slow: false,
            error: None,
        };
        assert_eq!(status_text(&row(true, true, true)), "Status: Running");
        assert_eq!(
            status_text(&row(true, false, true)),
            "Status: Running (restart to update)"
        );
        assert_eq!(status_text(&row(false, false, false)), "Status: Disabled");
        assert_eq!(
            status_text(&row(false, true, false)),
            "Status: Disabled (restart to update)"
        );
        // A plugin that faulted is not waiting for a restart.
        let mut faulted = row(false, true, true);
        faulted.error = Some("on_ui: access violation".into());
        assert_eq!(status_text(&faulted), "Status: Disabled (error)");
        let mut slow = row(true, true, true);
        slow.slow = true;
        assert_eq!(status_text(&slow), "Status: Running (slow callbacks)");
    }

    #[test]
    fn the_plugin_list_draws_cards_and_toggles() {
        let fake = FakeWidgets::default();
        fake.flip.borrow_mut().push("Enable".into());
        let mut rows = vec![
            PluginRow {
                id: "a".into(),
                name: "Alpha".into(),
                version: "1.0.0".into(),
                author: Some("Ann".into()),
                description: Some("Does a.".into()),
                enabled: true,
                enabled_at_launch: true,
                running: true,
                slow: false,
                error: None,
            },
            PluginRow {
                id: "b".into(),
                name: "b".into(),
                version: "2.0.0".into(),
                author: None,
                description: None,
                enabled: false,
                enabled_at_launch: true,
                running: false,
                slow: false,
                error: Some("E_LOAD: on_load returned -4".into()),
            },
        ];
        let simulation = vec!["a".to_owned()];
        let mut sections = Vec::new();
        let action = draw_plugin_list(
            &fake,
            [0xaa, 0xbb, 0xcc, 0xdd],
            &simulation,
            &mut rows,
            |_, i| sections.push(i),
        );
        // Every card's box flipped; the last flip wins as the reported action.
        assert_eq!(action, Action::Toggle(1));
        assert!(!rows[0].enabled && rows[1].enabled);
        assert_eq!(sections, vec![0], "only running plugins draw their section");
        let calls = fake.calls.borrow();
        assert_eq!(calls[1], "label \"Fingerprint: aabbccdd (a)\"");
        for line in [
            "label \"Alpha\"",
            "label \"Author:\"",
            "label \"Ann\"",
            "label \"1.0.0\"",
            "label \"Status: Running\"",
            "label \"Description: Does a.\"",
            "label \"Status: Disabled (error, restart to update)\"",
            "label \"Error: E_LOAD: on_load returned -4\"",
        ] {
            assert!(calls.iter().any(|c| c == line), "{line} missing: {calls:?}");
        }
        drop(calls);

        let fake = FakeWidgets::default();
        assert_eq!(
            draw_plugin_list(&fake, [0; 4], &[], &mut [], |_, _| {}),
            Action::None
        );
        let calls = fake.calls.borrow();
        assert!(calls.iter().any(|c| c.contains("No plugins installed")));
        assert!(calls.iter().any(|c| c.contains("00000000 (no plugins)")));
    }

    #[test]
    fn text_edit_edits_the_plugins_buffer_in_place() {
        let fake = FakeWidgets::default();
        fake.typed
            .borrow_mut()
            .insert("Colour".into(), "ff8800".into());
        let mut buf = *b"25e65f    ";
        let mut other = *b"abc ";
        with_frame(&fake, |ui| unsafe {
            assert_eq!((ui.text_edit)(ui.ctx, s("Colour"), buf.as_mut_ptr(), 10), 1);
            assert_eq!(&buf[..7], b"ff8800 ");
            assert_eq!((ui.text_edit)(ui.ctx, s("Other"), other.as_mut_ptr(), 4), 0);
            assert_eq!(&other, b"abc ");
            // Refused: null buffer, zero or oversize capacity.
            assert_eq!(
                (ui.text_edit)(ui.ctx, s("Colour"), core::ptr::null_mut(), 8),
                0
            );
            assert_eq!((ui.text_edit)(ui.ctx, s("Colour"), buf.as_mut_ptr(), 0), 0);
            assert_eq!(
                (ui.text_edit)(ui.ctx, s("Colour"), buf.as_mut_ptr(), MAX_EDIT as u32 + 1),
                0
            );
        });
    }

    #[test]
    fn an_unterminated_buffer_is_terminated_before_the_game_sees_it() {
        let fake = FakeWidgets::default();
        let mut buf = *b"abcd";
        with_frame(&fake, |ui| unsafe {
            (ui.text_edit)(ui.ctx, s("X"), buf.as_mut_ptr(), 4);
        });
        assert_eq!(buf[3], 0);
    }

    #[test]
    fn set_active_greys_the_rows_after_it_and_is_closed_at_the_end_of_the_frame() {
        let fake = FakeWidgets::default();
        with_frame(&fake, |ui| unsafe {
            (ui.set_active)(ui.ctx, 0);
            (ui.set_active)(ui.ctx, 0); // twice is still one grey-out
            (ui.label)(ui.ctx, s("greyed"));
            (ui.set_active)(ui.ctx, 1);
            (ui.set_active)(ui.ctx, 1);
            (ui.label)(ui.ctx, s("normal"));
            (ui.set_active)(ui.ctx, 0); // forgotten: the host closes it
        });
        let calls = fake.calls.borrow();
        let marks: Vec<&str> = calls
            .iter()
            .map(String::as_str)
            .filter(|c| c.starts_with("disable") || c.starts_with("label"))
            .collect();
        assert_eq!(
            marks,
            [
                "disable_begin",
                "label \"greyed\"",
                "disable_end",
                "label \"normal\"",
                "disable_begin",
                "disable_end"
            ]
        );
    }
}
