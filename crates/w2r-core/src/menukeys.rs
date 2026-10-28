//! Ctrl+C, Ctrl+X, Ctrl+V and Ctrl+A in the game's menu text boxes (the options screen, the plugin
//! manager's cards and the color editor's hex box).
//!
//! The game's Nuklear edit widget handles copy, cut, paste and select-all (the `0x40` clipboard
//! flag is in every host text box), but the game never reports those keys to Nuklear, and it gives
//! the menu context a paste function (0x0059ac20, set at 0x0059a71f) but no copy function. So, once
//! per frame before a host screen's widgets are drawn, [`feed`] marks the key "pressed" in the
//! context's key table on the frame Ctrl+key goes down, and installs a copy function that puts the
//! text on the Windows clipboard. Build 1.0.2.2818; see the constants for where each number is
//! read from.

/// Nuklear's key ids in this build (`nk_textedit_key`'s jump table at 0x005cdc10, and the
/// `nk_input_is_key_pressed` calls in `nk_do_edit` at 0x005d80d2..0x005d8228).
pub const KEY_COPY: usize = 7;
pub const KEY_CUT: usize = 8;
pub const KEY_PASTE: usize = 9;
pub const KEY_SELECT_ALL: usize = 23;

/// What one key does this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    Nothing,
    /// Ctrl+key went down: mark the key pressed (down, clicked once).
    Press,
    /// Still held after a press: clear the click again, so the action happens once even if
    /// nothing else clears it.
    Hold,
    /// Ctrl+key is no longer held and we had marked it down: mark it up.
    Release,
}

/// One key's state between frames: whether Ctrl+key was held last frame, and whether the host left
/// the key marked down in Nuklear's table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyState {
    held: bool,
    fed: bool,
}

impl KeyState {
    pub const fn new() -> KeyState {
        KeyState {
            held: false,
            fed: false,
        }
    }

    /// `held_now`: Ctrl and the key are both down, and the game window is in front.
    pub fn step(&mut self, held_now: bool) -> Feed {
        let feed = if held_now && !self.held {
            self.fed = true;
            Feed::Press
        } else if !held_now && self.fed {
            self.fed = false;
            Feed::Release
        } else if held_now && self.fed {
            Feed::Hold
        } else {
            Feed::Nothing
        };
        self.held = held_now;
        feed
    }
}

#[cfg(all(windows, target_arch = "x86"))]
pub use win::feed;

#[cfg(all(windows, target_arch = "x86"))]
mod win {
    use super::{Feed, KEY_COPY, KEY_CUT, KEY_PASTE, KEY_SELECT_ALL, KeyState};
    use core::ffi::{c_char, c_void};
    use std::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};

    /// `nk_context.input.keyboard.keys` starts the context: `nk_do_edit` gets `&ctx->input` as
    /// the context itself (0x005b2242). Each key is `{ int down; unsigned clicked; }`.
    const KEY_STRIDE: usize = 8;
    /// `nk_context.clip.copy` (`clip` at 0x208c: userdata, paste, copy; read at 0x005b22c5).
    const CTX_CLIP_COPY: usize = 0x2094;

    type CopyFn = unsafe extern "C" fn(*mut c_void, *const c_char, i32);

    thread_local! {
        static KEYS: Cell<[KeyState; 4]> = const { Cell::new([KeyState::new(); 4]) };
    }

    fn down(vk: i32) -> bool {
        // SAFETY: a plain key-state query.
        (unsafe { GetAsyncKeyState(vk) }) as u16 & 0x8000 != 0
    }

    /// Nuklear's copy callback: `len` bytes of the selection, not NUL-terminated.
    unsafe extern "C" fn copy_text(_user: *mut c_void, text: *const c_char, len: i32) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            if text.is_null() || len <= 0 {
                return;
            }
            // SAFETY: Nuklear passes `len` bytes of the edit's own string.
            let bytes = unsafe { core::slice::from_raw_parts(text.cast::<u8>(), len as usize) };
            crate::chatclick::copy_text(&String::from_utf8_lossy(bytes));
        }));
    }

    /// Call once per frame on the menu context `ctx`, before the screen's widgets are drawn.
    pub fn feed(ctx: *mut c_void) {
        if ctx.is_null() {
            return;
        }
        let base = ctx.cast::<u8>();
        // SAFETY: offsets into the game's nk_context (see the constants), on the game's thread.
        unsafe {
            let copy = base.add(CTX_CLIP_COPY).cast::<usize>();
            if copy.read_unaligned() == 0 {
                copy.write_unaligned(copy_text as CopyFn as usize);
            }
        }
        let armed = crate::replayplay::game_has_focus() && down(VK_CONTROL as i32);
        let keys = [
            (b'C', KEY_COPY),
            (b'X', KEY_CUT),
            (b'V', KEY_PASTE),
            (b'A', KEY_SELECT_ALL),
        ];
        KEYS.with(|cell| {
            let mut state = cell.get();
            for (k, &(vk, key)) in keys.iter().enumerate() {
                let feed = state[k].step(armed && down(i32::from(vk)));
                // SAFETY: `key` is below the table's 31 entries.
                let slot = unsafe { base.add(key * KEY_STRIDE) };
                match feed {
                    Feed::Press => unsafe {
                        slot.cast::<i32>().write_unaligned(1);
                        slot.add(4).cast::<u32>().write_unaligned(1);
                    },
                    Feed::Hold => unsafe { slot.add(4).cast::<u32>().write_unaligned(0) },
                    Feed::Release => unsafe {
                        slot.cast::<i32>().write_unaligned(0);
                        slot.add(4).cast::<u32>().write_unaligned(0);
                    },
                    Feed::Nothing => {}
                }
            }
            cell.set(state);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_is_fed_once_and_released_when_let_go() {
        let mut k = KeyState::new();
        assert_eq!(k.step(false), Feed::Nothing);
        assert_eq!(k.step(true), Feed::Press);
        assert_eq!(k.step(true), Feed::Hold); // held: the click is cleared, no second paste
        assert_eq!(k.step(true), Feed::Hold);
        assert_eq!(k.step(false), Feed::Release);
        assert_eq!(k.step(false), Feed::Nothing);
        assert_eq!(k.step(true), Feed::Press);
    }

    #[test]
    fn the_key_ids_match_this_nuklear() {
        assert_eq!(
            (KEY_COPY, KEY_CUT, KEY_PASTE, KEY_SELECT_ALL),
            (7, 8, 9, 23)
        );
    }
}
