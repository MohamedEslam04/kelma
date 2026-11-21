//! Tray icon and unread badge as ARGB32 pixmaps (StatusNotifierItem `IconPixmap` and
//! `OverlayIconPixmap`), drawn in code from the same shapes as `packaging/io.github.mohamedeslam04.kelma.svg`.

/// An ARGB32 image, one pixel as the bytes `[A, R, G, B]`, not premultiplied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pixmap {
    pub width: u32,
    pub height: u32,
    pub argb: Vec<u8>,
}

impl Pixmap {
    /// The pixels as `[R, G, B, A]` bytes (what Slint images and most toolkits take).
    pub fn to_rgba(&self) -> Vec<u8> {
        self.argb
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[a, r, g, b]| [r, g, b, a])
            .collect()
    }

    /// `[A, R, G, B]` of the pixel at `(x, y)`.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.argb[i],
            self.argb[i + 1],
            self.argb[i + 2],
            self.argb[i + 3],
        ]
    }
}

/// Sizes offered to the host, which picks the closest.
pub const SIZES: [u32; 4] = [22, 32, 48, 64];

/// Label inside the badge: `None` for 0 (hidden), the number for 1 to 99, `99+` above.
pub fn badge_label(count: u32) -> Option<String> {
    match count {
        0 => None,
        1..=99 => Some(count.to_string()),
        _ => Some("99+".to_string()),
    }
}

const PURPLE: [u8; 4] = [255, 0x5b, 0x5f, 0xc7];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const RED: [u8; 4] = [255, 0xe0, 0x24, 0x2f];
const CLEAR: [u8; 4] = [0, 0, 0, 0];
/// Samples per pixel edge when anti-aliasing.
const SS: u32 = 4;

fn in_round_rect(x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32) -> bool {
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

fn in_capsule(x: f32, y: f32, (x0, x1, cy): (f32, f32, f32), r: f32) -> bool {
    let cx = x.clamp(x0, x1);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

/// The colour of `packaging/io.github.mohamedeslam04.kelma.svg` at a point of its 128 x 128 canvas.
fn svg_color(x: f32, y: f32) -> [u8; 4] {
    if !in_round_rect(x, y, (0.0, 0.0, 128.0, 128.0), 28.0) {
        return CLEAR;
    }
    let bubble = in_round_rect(x, y, (30.0, 28.0, 98.0, 84.0), 10.0)
        // The tail: the triangle (44,84) (62,84) (44,100).
        || (x >= 44.0 && y >= 84.0 && y <= 84.0 + (62.0 - x) * 16.0 / 18.0);
    let lines =
        in_capsule(x, y, (46.0, 82.0, 50.0), 3.5) || in_capsule(x, y, (46.0, 68.0, 62.0), 3.5);
    if bubble && !lines { WHITE } else { PURPLE }
}

/// Average `SS` x `SS` samples per pixel of `shade(u, v)` over the unit square, in straight alpha.
fn rasterize(size: u32, shade: impl Fn(f32, f32) -> [u8; 4]) -> Pixmap {
    let mut argb = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut a, mut r, mut g, mut b) = (0u32, 0u32, 0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let v = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    let c = shade(u, v);
                    let ca = c[0] as u32;
                    a += ca;
                    r += c[1] as u32 * ca;
                    g += c[2] as u32 * ca;
                    b += c[3] as u32 * ca;
                }
            }
            let n = SS * SS;
            if a == 0 {
                argb.extend_from_slice(&CLEAR);
            } else {
                let un = |c: u32| ((c + a / 2) / a) as u8;
                argb.extend_from_slice(&[((a + n / 2) / n) as u8, un(r), un(g), un(b)]);
            }
        }
    }
    Pixmap {
        width: size,
        height: size,
        argb,
    }
}

/// The app icon (purple rounded square with a white chat bubble) at `size` x `size`.
pub fn app_icon(size: u32) -> Pixmap {
    rasterize(size, |u, v| svg_color(u * 128.0, v * 128.0))
}

/// 3 x 5 glyphs, one row per entry, bit 2 is the left column.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [7, 1, 7, 4, 7],
        '3' => [7, 1, 7, 1, 7],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 7, 1, 7],
        '6' => [7, 4, 7, 5, 7],
        '7' => [7, 1, 1, 1, 1],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 7],
        _ => [0, 2, 7, 2, 0], // '+'
    }
}

/// The red badge with the count in white, filling a `size` x `size` canvas; `None` for 0.
pub fn badge_overlay(count: u32, size: u32) -> Option<Pixmap> {
    let label = badge_label(count)?;
    let mut p = rasterize(size, |u, v| {
        let (dx, dy) = (u - 0.5, v - 0.5);
        if dx * dx + dy * dy <= 0.25 {
            RED
        } else {
            CLEAR
        }
    });
    // Glyph grid: 3 columns per glyph and 1 between, 5 rows; the biggest whole-pixel scale that
    // leaves a margin inside the disc.
    let glyphs = label.chars().count() as u32;
    let units_w = glyphs * 3 + (glyphs - 1);
    let scale = (((size as f32 * 0.72) as u32) / units_w)
        .min(((size as f32 * 0.6) as u32) / 5)
        .max(1);
    let x0 = size.saturating_sub(units_w * scale) / 2;
    let y0 = size.saturating_sub(5 * scale) / 2;
    for (i, ch) in label.chars().enumerate() {
        for (row, bits) in glyph(ch).into_iter().enumerate() {
            for col in 0..3u32 {
                if bits & (4 >> col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let x = x0 + (i as u32 * 4 + col) * scale + dx;
                        let y = y0 + row as u32 * scale + dy;
                        if x < size && y < size {
                            let at = ((y * size + x) * 4) as usize;
                            p.argb[at..at + 4].copy_from_slice(&WHITE);
                        }
                    }
                }
            }
        }
    }
    Some(p)
}

/// The app icon with the unread badge drawn over its top-right corner, as one picture: for trays
/// that take a single icon (Windows, macOS). Just the icon for 0.
pub fn badged_icon(count: u32, size: u32) -> Pixmap {
    let mut icon = app_icon(size);
    let b = size * 5 / 8;
    let Some(badge) = badge_overlay(count, b) else {
        return icon;
    };
    let x0 = size - b;
    for y in 0..b {
        for x in 0..b {
            let [sa, sr, sg, sb] = badge.pixel(x, y);
            if sa == 0 {
                continue;
            }
            let at = (((y * size) + x0 + x) * 4) as usize;
            let dst = &mut icon.argb[at..at + 4];
            // Straight-alpha "over".
            let (sa, da) = (f32::from(sa) / 255.0, f32::from(dst[0]) / 255.0);
            let oa = sa + da * (1.0 - sa);
            let mix = |s: u8, d: u8| {
                ((f32::from(s) * sa + f32::from(d) * da * (1.0 - sa)) / oa).round() as u8
            };
            let out = [
                (oa * 255.0).round() as u8,
                mix(sr, dst[1]),
                mix(sg, dst[2]),
                mix(sb, dst[3]),
            ];
            dst.copy_from_slice(&out);
        }
    }
    icon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_reorders_each_pixel() {
        let p = Pixmap {
            width: 1,
            height: 1,
            argb: vec![255, 1, 2, 3],
        };
        assert_eq!(p.to_rgba(), [1, 2, 3, 255]);
    }

    #[test]
    fn the_badged_icon_is_the_icon_with_a_red_corner() {
        assert_eq!(badged_icon(0, 32), app_icon(32));
        let p = badged_icon(3, 32);
        assert_eq!(p.argb.len(), 32 * 32 * 4);
        let corner = p.pixel(22, 1);
        assert_eq!(corner[0], 255);
        assert!(
            corner[1] > corner[2] && corner[1] > corner[3],
            "red: {corner:?}"
        );
        // The bottom-left of the icon is untouched.
        assert_eq!(p.pixel(4, 28), app_icon(32).pixel(4, 28));
    }

    const WHITE: [u8; 4] = [255, 255, 255, 255];

    #[test]
    fn label_zero_hides_one_to_99_shows_number_above_is_99_plus() {
        assert_eq!(badge_label(0), None);
        assert_eq!(badge_label(1).as_deref(), Some("1"));
        assert_eq!(badge_label(12).as_deref(), Some("12"));
        assert_eq!(badge_label(99).as_deref(), Some("99"));
        assert_eq!(badge_label(100).as_deref(), Some("99+"));
        assert_eq!(badge_label(u32::MAX).as_deref(), Some("99+"));
    }

    #[test]
    fn app_icon_has_the_right_shape_and_colors() {
        let p = app_icon(128);
        assert_eq!((p.width, p.height, p.argb.len()), (128, 128, 128 * 128 * 4));
        assert_eq!(p.pixel(0, 0)[0], 0, "corner is outside the rounded square");
        assert_eq!(
            p.pixel(10, 10),
            [255, 0x5b, 0x5f, 0xc7],
            "square is kelma purple"
        );
        assert_eq!(p.pixel(80, 40), WHITE, "bubble is white");
        assert_eq!(
            p.pixel(56, 62),
            [255, 0x5b, 0x5f, 0xc7],
            "text line is purple"
        );
    }

    #[test]
    fn app_icon_scales_to_every_offered_size() {
        for s in SIZES {
            let p = app_icon(s);
            assert_eq!(p.argb.len(), (s * s * 4) as usize);
            assert_eq!(p.pixel(s / 2, s / 6)[0], 255);
        }
    }

    #[test]
    fn badge_is_none_for_zero() {
        assert_eq!(badge_overlay(0, 32), None);
    }

    fn white_pixels(p: &Pixmap) -> usize {
        (0..p.height)
            .flat_map(|y| (0..p.width).map(move |x| (x, y)))
            .filter(|&(x, y)| p.pixel(x, y) == WHITE)
            .count()
    }

    #[test]
    fn badge_is_a_red_disc_with_white_digits() {
        let p = badge_overlay(3, 32).unwrap();
        assert_eq!((p.width, p.height, p.argb.len()), (32, 32, 32 * 32 * 4));
        assert_eq!(p.pixel(0, 0)[0], 0, "corner transparent");
        assert_eq!(p.pixel(31, 31)[0], 0);
        let edge = p.pixel(16, 2);
        assert_eq!(edge[0], 255, "inside the disc");
        assert!(edge[1] > edge[2] && edge[1] > edge[3], "red: {edge:?}");
        assert!(white_pixels(&p) > 10, "digit pixels drawn");
    }

    #[test]
    fn badge_renders_every_range_distinctly_at_every_size() {
        for size in SIZES {
            let one = badge_overlay(1, size).unwrap();
            let twelve = badge_overlay(12, size).unwrap();
            let capped = badge_overlay(99, size).unwrap();
            let plus = badge_overlay(100, size).unwrap();
            let again = badge_overlay(500, size).unwrap();
            assert_ne!(one, twelve);
            assert_ne!(twelve, capped);
            assert_ne!(capped, plus);
            assert_eq!(plus, again, "99+ for any larger count");
            for p in [&one, &twelve, &capped, &plus] {
                assert!(white_pixels(p) > 0, "size {size} has digits");
            }
        }
    }

    #[test]
    fn digits_stay_inside_the_disc() {
        let p = badge_overlay(100, 32).unwrap();
        for y in 0..32 {
            for x in 0..32 {
                if p.pixel(x, y) == WHITE {
                    let (dx, dy) = (x as f32 + 0.5 - 16.0, y as f32 + 0.5 - 16.0);
                    assert!(
                        dx * dx + dy * dy < 15.5 * 15.5,
                        "white pixel outside disc at {x},{y}"
                    );
                }
            }
        }
    }
}
