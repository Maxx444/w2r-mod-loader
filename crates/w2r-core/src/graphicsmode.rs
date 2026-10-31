//! Makes the HUD's classic/HD button remember its choice, as F5 does. x86 only.
//!
//! The game keeps the choice for the next launch in bit 0x10 of `GameSettingsFlags` (set: classic;
//! `FUN_006146c0` reads it when it creates the window). F5 (`FUN_004c8ab0`, scancode 0x3f) flips
//! the mode with `GraphicsToggle` and then updates that bit; the HUD button (`FUN_0052d930`) only
//! calls `GraphicsToggle`, so its choice is lost. Hooking `GraphicsToggle` itself keeps the bit in
//! step for both: when F5 runs its own update afterwards it writes the same value. The stock game
//! does this without the mod loader too.
//!
//! Leaving a match to the menu loses the choice too, for both F5 and the button, with or without
//! the mod loader (Maxx444's test, 2026-10-31). Where the game resets it is not known yet, so
//! this also remembers the last choice made with a toggle, logs every change of the mode and the
//! bit (lines start with "graphics:"), puts the bit back when something else changes it, and sets
//! it again just before `PersistSettings` writes Warcraft2.ini.

/// Bit of `GameSettingsFlags` that starts the next launch in classic graphics.
pub const CLASSIC_BIT: u32 = 0x10;
/// `GraphicsMode` after a toggle to classic. A toggle to HD leaves 3 (0 and 1 are the settled
/// classic and HD; 2 and 3 are the switches in progress).
pub const MODE_TO_CLASSIC: u32 = 2;

/// The player's last choice, from a toggle: none yet, classic or HD.
pub const CHOSEN_NONE: u32 = 0;
pub const CHOSEN_CLASSIC: u32 = 1;
pub const CHOSEN_HD: u32 = 2;

/// The choice a toggle that left the mode at `mode` stands for.
pub fn chosen_after(mode: u32) -> u32 {
    if mode == MODE_TO_CLASSIC {
        CHOSEN_CLASSIC
    } else {
        CHOSEN_HD
    }
}

/// The flags word with the next-launch bit set to `chosen`; `None` when it already matches or
/// nothing has been chosen.
pub fn flags_for(flags: u32, chosen: u32) -> Option<u32> {
    let want = match chosen {
        CHOSEN_CLASSIC => flags | CLASSIC_BIT,
        CHOSEN_HD => flags & !CLASSIC_BIT,
        _ => return None,
    };
    (want != flags).then_some(want)
}

/// For the log.
pub fn mode_text(mode: u32) -> &'static str {
    match mode {
        0 => "classic",
        1 => "HD",
        2 => "switching to classic",
        3 => "switching to HD",
        _ => "unknown",
    }
}

/// For the log: what the next launch starts in.
pub fn next_launch_text(flags: u32) -> &'static str {
    if flags & CLASSIC_BIT != 0 {
        "classic"
    } else {
        "HD"
    }
}

/// The flags word after a toggle that left the mode at `mode`.
pub fn flags_after(flags: u32, mode: u32) -> u32 {
    if mode == MODE_TO_CLASSIC {
        flags | CLASSIC_BIT
    } else {
        flags & !CLASSIC_BIT
    }
}

#[cfg(all(windows, target_arch = "x86"))]
pub use win::{install, watch};

#[cfg(all(windows, target_arch = "x86"))]
mod win {
    use core::ffi::c_void;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    use w2r_abi::{W2R_LOG_ERROR, W2R_LOG_INFO};

    use super::{CHOSEN_NONE, chosen_after, flags_after, flags_for, mode_text, next_launch_text};
    use crate::hooks::{self, Owner};
    use crate::host::Log;
    use crate::pe::Image;
    use crate::symbols::Symbols;

    const SYMBOL: &str = "GraphicsToggle";
    const PERSIST: &str = "PersistSettings";

    type ToggleFn = unsafe extern "C" fn() -> u8;
    type PersistFn = unsafe extern "C" fn();

    static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static PERSIST_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static MODE: AtomicUsize = AtomicUsize::new(0);
    static FLAGS: AtomicUsize = AtomicUsize::new(0);
    /// The player's last choice (`CHOSEN_*`), from a toggle this session.
    static CHOSEN: AtomicU32 = AtomicU32::new(CHOSEN_NONE);
    /// The mode and the bit last logged, `mode << 1 | bit`; `u32::MAX` before the first.
    static SEEN: AtomicU32 = AtomicU32::new(u32::MAX);

    fn line(text: &str) {
        if let Some(s) = crate::entry::startup() {
            s.log.line(W2R_LOG_INFO, "host", text);
        }
    }

    /// The game's mode word and flags word, once `install` found them.
    fn words() -> Option<(*const u32, *mut u32)> {
        let (mode, flags) = (MODE.load(Ordering::Relaxed), FLAGS.load(Ordering::Relaxed));
        (mode != 0 && flags != 0).then_some((mode as *const u32, flags as *mut u32))
    }

    fn seen(mode: u32, flags: u32) -> u32 {
        (mode << 1) | u32::from(flags & super::CLASSIC_BIT != 0)
    }

    unsafe extern "C" fn detour() -> u8 {
        let original: ToggleFn = unsafe { core::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
        let changed = unsafe { original() };
        if changed != 0 {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                if let Some((mode, flags)) = words() {
                    // SAFETY: the game's mode word and settings flags, on the game's thread.
                    unsafe {
                        let mode = mode.read_volatile();
                        let after = flags_after(flags.read_volatile(), mode);
                        flags.write_volatile(after);
                        CHOSEN.store(chosen_after(mode), Ordering::Relaxed);
                        SEEN.store(seen(mode, after), Ordering::Relaxed);
                        line(&format!(
                            "graphics: toggled ({}); next launch {}",
                            mode_text(mode),
                            next_launch_text(after)
                        ));
                    }
                }
            }));
        }
        changed
    }

    /// `PersistSettings` (writes Warcraft2.ini): sets the next-launch bit to the player's last
    /// choice first, so the file gets it.
    unsafe extern "C" fn persist_detour() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let Some((mode, flags)) = words() else { return };
            // SAFETY: the game's mode word and settings flags, on the game's thread.
            unsafe {
                let (mode, before) = (mode.read_volatile(), flags.read_volatile());
                match flags_for(before, CHOSEN.load(Ordering::Relaxed)) {
                    Some(after) => {
                        flags.write_volatile(after);
                        SEEN.store(seen(mode, after), Ordering::Relaxed);
                        line(&format!(
                            "graphics: saving settings; next launch was {}, set back to {} (the last choice)",
                            next_launch_text(before),
                            next_launch_text(after)
                        ));
                    }
                    None => line(&format!(
                        "graphics: saving settings; next launch {} (mode {})",
                        next_launch_text(before),
                        mode_text(mode)
                    )),
                }
            }
        }));
        let original: PersistFn =
            unsafe { core::mem::transmute(PERSIST_ORIGINAL.load(Ordering::Acquire)) };
        unsafe { original() };
    }

    /// Once per UI window, from the frame hook: logs a change of the mode or the bit, and when the
    /// bit changed by something other than a toggle, puts the player's last choice back.
    pub fn watch() {
        let Some((mode, flags)) = words() else { return };
        // SAFETY: the game's mode word and settings flags, on the game's thread.
        unsafe {
            let (m, f) = (mode.read_volatile(), flags.read_volatile());
            let now = seen(m, f);
            let before = SEEN.swap(now, Ordering::Relaxed);
            if now == before {
                return;
            }
            line(&format!(
                "graphics: mode {}; next launch {}",
                mode_text(m),
                next_launch_text(f)
            ));
            // Only a settled mode, and only a bit change (a toggle already updated `SEEN`).
            let bit_changed = before != u32::MAX && (before ^ now) & 1 != 0;
            if m <= 1
                && bit_changed
                && let Some(fixed) = flags_for(f, CHOSEN.load(Ordering::Relaxed))
            {
                flags.write_volatile(fixed);
                SEEN.store(seen(m, fixed), Ordering::Relaxed);
                line(&format!(
                    "graphics: next launch changed to {} without a toggle; set back to {} (the last choice)",
                    next_launch_text(f),
                    next_launch_text(fixed)
                ));
            }
        }
    }

    fn hook_persist(image: &Image, symbols: &Symbols, log: &dyn Log) {
        let Some(target) = symbols.address(PERSIST) else {
            log.line(
                W2R_LOG_ERROR,
                "host",
                "graphics: PersistSettings is missing",
            );
            return;
        };
        // SAFETY: `persist_detour` matches PersistSettings' cdecl `void()` signature.
        let result = unsafe {
            hooks::global().install(
                Owner::Host,
                image,
                target,
                persist_detour as *mut c_void,
                &format!("host:{PERSIST}"),
                |original| PERSIST_ORIGINAL.store(original as usize, Ordering::Release),
            )
        };
        match result {
            Ok(_) => log.line(
                W2R_LOG_INFO,
                "host",
                &format!("hooked {PERSIST} @ {target:#010x} (keeps the graphics choice)"),
            ),
            Err(e) => log.line(
                W2R_LOG_ERROR,
                "host",
                &format!("could not hook {PERSIST}: {e:?}"),
            ),
        }
    }

    pub fn install(image: &Image, symbols: &Symbols, log: &dyn Log) -> bool {
        let (Some(target), Some(mode), Some(flags)) = (
            symbols.address(SYMBOL),
            symbols.address("GraphicsMode"),
            symbols.address("GameSettingsFlags"),
        ) else {
            log.line(
                W2R_LOG_ERROR,
                "host",
                "graphics button: a symbol is missing; the button stays as it is",
            );
            return false;
        };
        MODE.store(mode, Ordering::Release);
        FLAGS.store(flags, Ordering::Release);
        // SAFETY: `detour` matches GraphicsToggle's cdecl `unsigned char()` signature (build 2818:
        // no arguments, the changed flag in AL).
        let result = unsafe {
            hooks::global().install(
                Owner::Host,
                image,
                target,
                detour as *mut c_void,
                &format!("host:{SYMBOL}"),
                |original| ORIGINAL.store(original as usize, Ordering::Release),
            )
        };
        match result {
            Ok(_) => {
                log.line(
                    W2R_LOG_INFO,
                    "host",
                    &format!("hooked {SYMBOL} @ {target:#010x} (the HUD button saves its choice)"),
                );
                hook_persist(image, symbols, log);
                true
            }
            Err(e) => {
                log.line(
                    W2R_LOG_ERROR,
                    "host",
                    &format!("could not hook {SYMBOL}: {e:?}"),
                );
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toggle_to_classic_sets_the_bit_and_to_hd_clears_it() {
        assert_eq!(flags_after(0x07, MODE_TO_CLASSIC), 0x17);
        assert_eq!(flags_after(0x17, 3), 0x07);
        assert_eq!(flags_after(0x17, MODE_TO_CLASSIC), 0x17);
        assert_eq!(flags_after(0x07, 3), 0x07);
    }

    #[test]
    fn the_last_choice_is_put_back_only_when_it_differs() {
        assert_eq!(chosen_after(MODE_TO_CLASSIC), CHOSEN_CLASSIC);
        assert_eq!(chosen_after(3), CHOSEN_HD);
        assert_eq!(flags_for(0x07, CHOSEN_CLASSIC), Some(0x17));
        assert_eq!(flags_for(0x17, CHOSEN_HD), Some(0x07));
        assert_eq!(flags_for(0x17, CHOSEN_CLASSIC), None);
        assert_eq!(flags_for(0x07, CHOSEN_HD), None);
        assert_eq!(flags_for(0x17, CHOSEN_NONE), None);
        assert_eq!(next_launch_text(0x17), "classic");
        assert_eq!(next_launch_text(0x07), "HD");
        assert_eq!(mode_text(1), "HD");
    }
}
