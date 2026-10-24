//! Colors for the options screen's color picker (docs/settings-screen.md): the `rrggbb` form a
//! color setting is stored in, conversion between RGB and hue / saturation / brightness, and the
//! pixels of the picker's two gradients. No game access, so it is tested everywhere.

/// An 8-bit RGB color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Hue in degrees (0 to 360), saturation and brightness from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

/// Exactly six hex digits, without `#`.
pub fn parse_hex(text: &str) -> Option<Rgb> {
    if text.len() != 6 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(text, 16).ok()?;
    Some(Rgb {
        r: (n >> 16) as u8,
        g: (n >> 8) as u8,
        b: n as u8,
    })
}

/// Six lowercase hex digits.
pub fn to_hex(c: Rgb) -> String {
    format!("{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// `text` as stored: lowercase hex, or `None` if it is not six hex digits.
pub fn normalize(text: &str) -> Option<String> {
    parse_hex(text).map(to_hex)
}

/// The color as Nuklear packs it: `0xAABBGGRR`, opaque.
pub fn nk_color(c: Rgb) -> u32 {
    0xff00_0000 | (u32::from(c.b) << 16) | (u32::from(c.g) << 8) | u32::from(c.r)
}

pub fn to_hsv(c: Rgb) -> Hsv {
    let r = f32::from(c.r) / 255.0;
    let g = f32::from(c.g) / 255.0;
    let b = f32::from(c.b) / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    Hsv { h, s, v: max }
}

pub fn to_rgb(c: Hsv) -> Rgb {
    let h = c.h.rem_euclid(360.0) / 60.0;
    let s = c.s.clamp(0.0, 1.0);
    let v = c.v.clamp(0.0, 1.0);
    let chroma = v * s;
    let x = chroma * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = v - chroma;
    let byte = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb {
        r: byte(r),
        g: byte(g),
        b: byte(b),
    }
}

/// RGBA pixels of the saturation / brightness square for `hue`, `side` by `side`: saturation
/// grows to the right, brightness grows upwards.
pub fn square_pixels(hue: f32, side: usize) -> Vec<u8> {
    let last = (side.max(2) - 1) as f32;
    let mut out = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        for x in 0..side {
            let c = to_rgb(Hsv {
                h: hue,
                s: x as f32 / last,
                v: 1.0 - y as f32 / last,
            });
            out.extend_from_slice(&[c.r, c.g, c.b, 255]);
        }
    }
    out
}

/// RGBA pixels of the hue strip, `width` by `height`: hue 0 at the top, 360 at the bottom.
pub fn hue_pixels(width: usize, height: usize) -> Vec<u8> {
    let last = (height.max(2) - 1) as f32;
    let mut out = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let c = to_rgb(Hsv {
            h: 360.0 * y as f32 / last,
            s: 1.0,
            v: 1.0,
        });
        for _ in 0..width {
            out.extend_from_slice(&[c.r, c.g, c.b, 255]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_is_strict() {
        assert_eq!(
            parse_hex("25E65f"),
            Some(Rgb {
                r: 0x25,
                g: 0xe6,
                b: 0x5f
            })
        );
        assert_eq!(normalize("25E65F").as_deref(), Some("25e65f"));
        for bad in ["", "#25e65f", "25e65", "25e65fa", "25e6 5", "zzzzzz"] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn hsv_round_trips_every_primary_and_a_few_others() {
        for hex in [
            "ff0000", "00ff00", "0000ff", "ffff00", "00ffff", "ff00ff", "ffffff", "000000",
            "808080", "25e65f", "929292", "69a7ff", "e87bdd", "00c6e0",
        ] {
            let c = parse_hex(hex).unwrap();
            assert_eq!(to_hex(to_rgb(to_hsv(c))), hex);
        }
    }

    #[test]
    fn hsv_values_are_the_usual_ones() {
        let h = to_hsv(parse_hex("00ff00").unwrap());
        assert_eq!((h.h, h.s, h.v), (120.0, 1.0, 1.0));
        let grey = to_hsv(parse_hex("808080").unwrap());
        assert_eq!(grey.s, 0.0);
        assert_eq!(
            to_rgb(Hsv {
                h: 360.0,
                s: 1.0,
                v: 1.0
            }),
            parse_hex("ff0000").unwrap()
        );
    }

    #[test]
    fn nuklear_packs_red_lowest() {
        assert_eq!(nk_color(parse_hex("112233").unwrap()), 0xff33_2211);
    }

    #[test]
    fn the_square_runs_white_to_hue_across_and_black_at_the_bottom() {
        let side = 8;
        let px = square_pixels(0.0, side);
        assert_eq!(px.len(), side * side * 4);
        let at = |x: usize, y: usize| &px[(y * side + x) * 4..(y * side + x) * 4 + 4];
        assert_eq!(at(0, 0), &[255, 255, 255, 255]);
        assert_eq!(at(side - 1, 0), &[255, 0, 0, 255]);
        assert_eq!(at(0, side - 1), &[0, 0, 0, 255]);
        assert_eq!(at(side - 1, side - 1), &[0, 0, 0, 255]);
    }

    #[test]
    fn the_hue_strip_starts_and_ends_red() {
        let px = hue_pixels(2, 7);
        assert_eq!(px.len(), 2 * 7 * 4);
        assert_eq!(&px[..4], &[255, 0, 0, 255]);
        assert_eq!(&px[px.len() - 4..], &[255, 0, 0, 255]);
        assert_eq!(
            &px[3 * 8..3 * 8 + 4],
            &[0, 255, 255, 255],
            "the middle is cyan"
        );
    }
}
