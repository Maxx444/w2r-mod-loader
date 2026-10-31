//! Makes the HUD's classic/HD button remember its choice, as F5 does. x86 only.
//!
//! The game keeps the choice for the next launch in bit 0x10 of `GameSettingsFlags` (set: classic;
//! `FUN_006146c0` reads it when it creates the window). F5 (`FUN_004c8ab0`, scancode 0x3f) flips
//! the mode with `GraphicsToggle` and then updates that bit; the HUD button (`FUN_0052d930`) only
//! calls `GraphicsToggle`, so its choice is lost. Hooking `GraphicsToggle` itself keeps the bit in
//! step for both: when F5 runs its own update afterwards it writes the same value. The stock game
//! does this without the mod loader too.

/// Bit of `GameSettingsFlags` that starts the next launch in classic graphics.
pub const CLASSIC_BIT: u32 = 0x10;
/// `GraphicsMode` after a toggle to classic. A toggle to HD leaves 3 (0 and 1 are the settled
/// classic and HD; 2 and 3 are the switches in progress).
pub const MODE_TO_CLASSIC: u32 = 2;

/// The flags word after a toggle that left the mode at `mode`.
pub fn flags_after(flags: u32, mode: u32) -> u32 {
    if mode == MODE_TO_CLASSIC {
        flags | CLASSIC_BIT
    } else {
        flags & !CLASSIC_BIT
    }
}

#[cfg(all(windows, target_arch = "x86"))]
pub use win::install;

#[cfg(all(windows, target_arch = "x86"))]
mod win {
    use core::ffi::c_void;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use w2r_abi::{W2R_LOG_ERROR, W2R_LOG_INFO};

    use super::flags_after;
    use crate::hooks::{self, Owner};
    use crate::host::Log;
    use crate::pe::Image;
    use crate::symbols::Symbols;

    const SYMBOL: &str = "GraphicsToggle";

    type ToggleFn = unsafe extern "C" fn() -> u8;

    static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static MODE: AtomicUsize = AtomicUsize::new(0);
    static FLAGS: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn detour() -> u8 {
        let original: ToggleFn = unsafe { core::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
        let changed = unsafe { original() };
        if changed != 0 {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let (mode, flags) = (MODE.load(Ordering::Relaxed), FLAGS.load(Ordering::Relaxed));
                if mode != 0 && flags != 0 {
                    // SAFETY: the game's mode word and settings flags, on the game's thread.
                    unsafe {
                        let mode = (mode as *const u32).read_volatile();
                        let flags = flags as *mut u32;
                        flags.write_volatile(flags_after(flags.read_volatile(), mode));
                    }
                }
            }));
        }
        changed
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
}
