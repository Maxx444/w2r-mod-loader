//! Score screen bars in the team colors in use (hastinbe/w2r-mod-loader#3, item 4).
//!
//! After a match each player's row of numbers sits on bars drawn with the menu skins
//! `fe_endgame_stats_bar_0` .. `_7` of `Data\skins\skins.json`, one per team color: `progress` (the
//! dark half) and `progress_cursor` (the bright half), plain RGBA. They are read once at startup,
//! so a plugin that recolors the teams (the HD table `TeamColourRgba`) leaves them stock.
//!
//! The game builds the menu style from a skin record with `SkinApplyStyle` (0x0063c190) whenever a
//! skin is pushed. The host hooks it: the first time each bar record passes, it is known by its
//! stock colors; from then on, before the style is built, its two colors are set from the team's
//! entry in `TeamColourRgba` (bright half = the color, dark half = half of it, as in the stock
//! skins) when that entry differs from the game's own, and put back otherwise. Build 1.0.2.2818;
//! the record layout is in the constants below (read from the skins.json parser at 0x0063c0f0 and
//! 0x00640030).

/// Team colors, as many as bar skins.
pub const TEAMS: usize = 8;

/// The stock bright halves (`progress_cursor`) of `fe_endgame_stats_bar_0` .. `_7`.
pub const STOCK_CURSOR: [[u8; 3]; TEAMS] = [
    [164, 0, 0],
    [0, 60, 192],
    [44, 180, 148],
    [156, 72, 176],
    [240, 132, 20],
    [40, 40, 60],
    [224, 224, 224],
    [252, 252, 72],
];

/// The stock dark halves (`progress`): each is half its bright half.
pub const STOCK_PROGRESS: [[u8; 3]; TEAMS] = [
    [82, 0, 0],
    [0, 30, 96],
    [22, 90, 74],
    [78, 36, 88],
    [120, 66, 10],
    [20, 20, 30],
    [112, 112, 112],
    [126, 126, 36],
];

/// Which bar skin a record is, from its two colors as loaded (RGBA). None for any other skin.
pub fn identify(progress: [u8; 4], cursor: [u8; 4]) -> Option<usize> {
    (0..TEAMS).find(|&k| progress == rgba(STOCK_PROGRESS[k]) && cursor == rgba(STOCK_CURSOR[k]))
}

fn rgba([r, g, b]: [u8; 3]) -> [u8; 4] {
    [r, g, b, 255]
}

/// A 0..1 color channel as a byte.
pub fn byte_of(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The two colors bar `k` should have (dark, bright): stock while the team's HD color is the game's
/// own (`now` equals `stock`, within rounding), else built from `now`.
pub fn bar_colors(k: usize, now: [f32; 4], stock: [f32; 4]) -> ([u8; 4], [u8; 4]) {
    let same = (0..3).all(|c| byte_of(now[c]) == byte_of(stock[c]));
    if same || k >= TEAMS {
        let k = k.min(TEAMS - 1);
        return (rgba(STOCK_PROGRESS[k]), rgba(STOCK_CURSOR[k]));
    }
    let bright = [byte_of(now[0]), byte_of(now[1]), byte_of(now[2])];
    (rgba(bright.map(|v| v / 2)), rgba(bright))
}

#[cfg(all(windows, target_arch = "x86"))]
pub use win::install;

#[cfg(all(windows, target_arch = "x86"))]
mod win {
    use super::{TEAMS, bar_colors, identify};
    use core::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock};

    use w2r_abi::{W2R_LOG_ERROR, W2R_LOG_INFO, W2R_LOG_WARN};

    use crate::guard::guarded;
    use crate::hooks::{self, Owner};
    use crate::host::Log;
    use crate::pe::Image;
    use crate::symbols::Symbols;

    const SYMBOL: &str = "SkinApplyStyle";
    /// A skin record's `progress` element (0x63c0ea) and `progress_cursor` element (0x63c104),
    /// plus the element's `color` (RGBA bytes, 0x6401d6).
    const PROGRESS_COLOR: usize = 0x10f8 + 0x40;
    const CURSOR_COLOR: usize = 0x115c + 0x40;
    /// Floats per `TeamColourRgba` entry.
    const TABLE_STRIDE: usize = 4;

    type ApplyFn = unsafe extern "C" fn(*mut c_void, *mut u8) -> u32;

    static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    struct Game {
        table: usize,
        /// `TeamColourRgba` as the game set it, read when the hook went in (before any plugin
        /// wrote it).
        stock: [[f32; 4]; TEAMS],
    }
    static GAME: OnceLock<Game> = OnceLock::new();
    /// The bar records seen so far, with their team: once recolored, a record no longer shows
    /// its stock colors.
    static KNOWN: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

    unsafe fn read4(at: usize) -> [u8; 4] {
        unsafe { core::ptr::read_unaligned(at as *const [u8; 4]) }
    }

    unsafe fn table_entry(table: usize, k: usize) -> [f32; 4] {
        unsafe { core::ptr::read_unaligned((table as *const f32).add(k * TABLE_STRIDE).cast()) }
    }

    /// Sets a bar record's colors before the style is built from it. Any other record is left
    /// alone.
    unsafe fn recolor(record: usize) {
        let Some(game) = GAME.get() else { return };
        let Ok(mut known) = KNOWN.lock() else { return };
        let k = match known.iter().find(|(r, _)| *r == record) {
            Some(&(_, k)) => k,
            None => {
                // SAFETY: a skin record (0x1224+ bytes) the game is about to read.
                let (p, c) =
                    unsafe { (read4(record + PROGRESS_COLOR), read4(record + CURSOR_COLOR)) };
                let Some(k) = identify(p, c) else { return };
                known.push((record, k));
                k
            }
        };
        // SAFETY: the game's HD color table, eight entries of four floats.
        let now = unsafe { table_entry(game.table, k) };
        let (dark, bright) = bar_colors(k, now, game.stock[k]);
        // SAFETY: the record's two color fields, on the game's thread.
        unsafe {
            if read4(record + PROGRESS_COLOR) != dark {
                core::ptr::write_unaligned((record + PROGRESS_COLOR) as *mut [u8; 4], dark);
            }
            if read4(record + CURSOR_COLOR) != bright {
                core::ptr::write_unaligned((record + CURSOR_COLOR) as *mut [u8; 4], bright);
            }
        }
    }

    unsafe extern "C" fn apply_detour(ctx: *mut c_void, record: *mut u8) -> u32 {
        if !record.is_null() {
            if let Err(fault) = guarded(|| unsafe { recolor(record as usize) }) {
                crate::session::host_fault("score screen bars", &fault);
            }
        }
        let original: ApplyFn = unsafe { core::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
        unsafe { original(ctx, record) }
    }

    pub fn install(image: &Image, symbols: &Symbols, log: &dyn Log) -> bool {
        let (Some(target), Some(table)) =
            (symbols.address(SYMBOL), symbols.address("TeamColourRgba"))
        else {
            log.line(W2R_LOG_ERROR, "host", "score screen bars: symbols missing");
            return false;
        };
        let mut stock = [[0f32; 4]; TEAMS];
        for (k, entry) in stock.iter_mut().enumerate() {
            // SAFETY: the game's HD color table, eight entries of four floats.
            *entry = unsafe { table_entry(table, k) };
        }
        let _ = GAME.set(Game { table, stock });
        // SAFETY: `apply_detour` matches SkinApplyStyle's cdecl (ctx, record) signature (build 2818;
        // callers push two arguments and clean them up), and passes the return value through.
        let result = unsafe {
            hooks::global().install(
                Owner::Host,
                image,
                target,
                apply_detour as *mut c_void,
                &format!("host:{SYMBOL}"),
                |original| ORIGINAL.store(original as usize, Ordering::Release),
            )
        };
        match result {
            Ok(_) => {
                log.line(
                    W2R_LOG_INFO,
                    "host",
                    "score screen bars follow the team colors in use",
                );
                true
            }
            Err(e) => {
                log.line(
                    W2R_LOG_WARN,
                    "host",
                    &format!("could not hook {SYMBOL} (score screen bars stay stock): {e:?}"),
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
    fn the_stock_dark_half_is_half_the_bright_half() {
        for k in 0..TEAMS {
            assert_eq!(STOCK_CURSOR[k].map(|v| v / 2), STOCK_PROGRESS[k], "bar {k}");
        }
    }

    #[test]
    fn bars_are_known_by_their_stock_colors_only() {
        assert_eq!(identify([82, 0, 0, 255], [164, 0, 0, 255]), Some(0));
        assert_eq!(identify([126, 126, 36, 255], [252, 252, 72, 255]), Some(7));
        assert_eq!(identify([82, 0, 0, 255], [0, 60, 192, 255]), None);
        assert_eq!(identify([0; 4], [0; 4]), None);
        assert_eq!(identify([82, 0, 0, 128], [164, 0, 0, 255]), None);
    }

    #[test]
    fn a_recolored_team_gets_its_color_and_half_of_it() {
        let stock = [0.0, 150.0 / 255.0, 1.0, 1.0];
        let blue = [0.0, 60.0 / 255.0, 192.0 / 255.0, 1.0];
        assert_eq!(
            bar_colors(1, blue, stock),
            ([0, 30, 96, 255], [0, 60, 192, 255])
        );
        let pink = [1.0, 0.0, 1.0, 1.0];
        assert_eq!(
            bar_colors(5, pink, [0.16, 0.16, 0.24, 1.0]),
            ([127, 0, 127, 255], [255, 0, 255, 255])
        );
    }

    #[test]
    fn the_games_own_color_keeps_the_stock_bar() {
        let stock = [0.0, 150.0 / 255.0, 1.0, 1.0];
        assert_eq!(
            bar_colors(1, stock, stock),
            ([0, 30, 96, 255], [0, 60, 192, 255])
        );
        // Rounding noise is still "the game's own".
        let near = [0.0001, 150.0 / 255.0 + 0.0005, 0.9999, 1.0];
        assert_eq!(
            bar_colors(1, near, stock),
            ([0, 30, 96, 255], [0, 60, 192, 255])
        );
    }

    #[test]
    fn channels_round_and_clamp() {
        assert_eq!(byte_of(0.0), 0);
        assert_eq!(byte_of(1.0), 255);
        assert_eq!(byte_of(0.5), 128);
        assert_eq!(byte_of(-1.0), 0);
        assert_eq!(byte_of(2.0), 255);
    }
}
