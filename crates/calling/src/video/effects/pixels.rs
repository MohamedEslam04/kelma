//! The pixel work of the effects, on I420 planes: lookup tables for the filters and the low-light
//! boost, a box blur, the person mask (scaled, smoothed) and mixing two pictures through it.

use super::Filter;
use crate::video::{I420Frame, RgbFrame};

/// A new value for every 8-bit sample.
pub type Lut = [u8; 256];

pub fn apply_lut(plane: &mut [u8], lut: &Lut) {
    for p in plane {
        *p = lut[usize::from(*p)];
    }
}

fn table(f: impl Fn(i32) -> i32) -> Lut {
    let mut t = [0u8; 256];
    for (i, v) in t.iter_mut().enumerate() {
        *v = f(i as i32).clamp(0, 255) as u8;
    }
    t
}

/// The low-light boost: the luma's shadows and mid-tones lifted (limited range, 16 black, 235
/// white), black and white kept.
pub fn brighten_lut() -> Lut {
    table(|i| {
        if !(16..=235).contains(&i) {
            return i;
        }
        let t = (i - 16) as f32 / 219.0;
        (16.0 + 219.0 * t.powf(0.65)).round() as i32
    })
}

/// The Y, U and V tables of a colour filter; `None` for no filter.
pub fn filter_luts(filter: Filter) -> Option<[Lut; 3]> {
    let same = table(|v| v);
    let shift = |d: i32| table(move |v| v + d);
    let scale = |k: f32| table(move |v| 128 + ((v - 128) as f32 * k).round() as i32);
    Some(match filter {
        Filter::None => return None,
        // Less blue, more red.
        Filter::Warm => [same, shift(-10), shift(9)],
        Filter::Cool => [same, shift(11), shift(-8)],
        Filter::Mono => [same, table(|_| 128), table(|_| 128)],
        // More contrast and colour.
        Filter::Vivid => [scale(1.1), scale(1.45), scale(1.45)],
    })
}

/// Blurs `src` (`w` x `h`) into `out`: a box of radius `r` run twice (close to a Gaussian), the
/// edges repeated. `tmp` is scratch space.
pub fn blur(src: &[u8], w: usize, h: usize, r: usize, out: &mut Vec<u8>, tmp: &mut Vec<u8>) {
    out.clear();
    out.extend_from_slice(&src[..w * h]);
    if r == 0 || w == 0 || h == 0 {
        return;
    }
    tmp.clear();
    tmp.resize(w * h, 0);
    for _ in 0..2 {
        box_rows(out, tmp, w, r);
        box_cols(tmp, out, w, h, r);
    }
}

/// One horizontal box pass of `src` into `dst`.
fn box_rows(src: &[u8], dst: &mut [u8], w: usize, r: usize) {
    let n = (2 * r + 1) as u32;
    let r = r as isize;
    for (row, out) in src.chunks_exact(w).zip(dst.chunks_exact_mut(w)) {
        let at = move |i: isize| u32::from(row[i.clamp(0, w as isize - 1) as usize]);
        let mut sum: u32 = (-r..=r).map(at).sum();
        for (x, o) in out.iter_mut().enumerate() {
            *o = ((sum + n / 2) / n) as u8;
            sum = sum + at(x as isize + r + 1) - at(x as isize - r);
        }
    }
}

/// One vertical box pass of `src` into `dst`, row by row (a running sum per column).
fn box_cols(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let n = (2 * r + 1) as u32;
    let r = r as isize;
    let row = move |y: isize| {
        let y = y.clamp(0, h as isize - 1) as usize;
        &src[y * w..(y + 1) * w]
    };
    let mut sums = vec![0u32; w];
    for y in -r..=r {
        for (s, v) in sums.iter_mut().zip(row(y)) {
            *s += u32::from(*v);
        }
    }
    for (y, out) in dst.chunks_exact_mut(w).enumerate().take(h) {
        for (o, s) in out.iter_mut().zip(&sums) {
            *o = ((s + n / 2) / n) as u8;
        }
        let (add, sub) = (row(y as isize + r + 1), row(y as isize - r));
        for ((s, a), b) in sums.iter_mut().zip(add).zip(sub) {
            *s = *s + u32::from(*a) - u32::from(*b);
        }
    }
}

/// How much of each pixel is the person: 255 the person, 0 the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Mask {
    /// The model's person probabilities as a mask with a short soft edge: below 0.25 is
    /// background, above 0.75 the person (a stray low value does not show the room through).
    pub fn from_probabilities(p: &[f32], width: usize, height: usize) -> Mask {
        let data = p
            .iter()
            .take(width * height)
            .map(|&v| {
                let t = ((v - 0.25) / 0.5).clamp(0.0, 1.0);
                (t * t * (3.0 - 2.0 * t) * 255.0).round() as u8
            })
            .collect();
        Mask {
            width,
            height,
            data,
        }
    }
}

/// Mixes `next` into `prev` (mostly `next`): the edge flickers less from frame to frame.
pub fn smooth(prev: &mut Mask, next: &Mask) {
    if (prev.width, prev.height) != (next.width, next.height) {
        *prev = next.clone();
        return;
    }
    for (p, n) in prev.data.iter_mut().zip(&next.data) {
        *p = ((u16::from(*p) + 3 * u16::from(*n) + 2) / 4) as u8;
    }
}

/// `mask` scaled to `w` x `h` (bilinear) into `out`.
pub fn upscale(mask: &Mask, w: usize, h: usize, out: &mut Vec<u8>) {
    out.clear();
    out.resize(w * h, 0);
    let (mw, mh) = (mask.width.max(1), mask.height.max(1));
    if mask.data.len() < mw * mh {
        return;
    }
    // The two source samples around a destination pixel's centre, and the second one's weight
    // in 256ths.
    let around = |i: usize, src: usize, dst: usize| {
        let f = ((i as f32 + 0.5) * src as f32 / dst as f32 - 0.5).max(0.0);
        let i0 = (f as usize).min(src - 1);
        let i1 = (i0 + 1).min(src - 1);
        (i0, i1, ((f - i0 as f32) * 256.0) as u32)
    };
    let xs: Vec<_> = (0..w).map(|x| around(x, mw, w)).collect();
    for (y, line) in out.chunks_exact_mut(w).enumerate() {
        let (y0, y1, fy) = around(y, mh, h);
        let (r0, r1) = (
            &mask.data[y0 * mw..(y0 + 1) * mw],
            &mask.data[y1 * mw..(y1 + 1) * mw],
        );
        for (o, &(x0, x1, fx)) in line.iter_mut().zip(&xs) {
            let top = u32::from(r0[x0]) * (256 - fx) + u32::from(r0[x1]) * fx;
            let bottom = u32::from(r1[x0]) * (256 - fx) + u32::from(r1[x1]) * fx;
            *o = ((top * (256 - fy) + bottom * fy + 32768) >> 16) as u8;
        }
    }
}

/// The chroma planes' mask: every other sample of every other row of the luma's.
pub fn half(alpha: &[u8], w: usize, h: usize, out: &mut Vec<u8>) {
    out.clear();
    for row in alpha.chunks_exact(w).take(h).step_by(2) {
        out.extend(row.iter().step_by(2));
    }
}

/// `fg` where `alpha` is 255, `bg` where it is 0, mixed in between.
pub fn composite(fg: &mut [u8], bg: &[u8], alpha: &[u8]) {
    for ((f, &b), &a) in fg.iter_mut().zip(bg).zip(alpha) {
        let a = u32::from(a);
        *f = ((u32::from(*f) * a + u32::from(b) * (255 - a) + 127) / 255) as u8;
    }
}

/// BT.601 limited range, as cameras write it.
pub fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = (i32::from(y) - 16).max(0) * 298;
    let (d, e) = (i32::from(u) - 128, i32::from(v) - 128);
    let clamp = |x: i32| (x >> 8).clamp(0, 255) as u8;
    [
        clamp(c + 409 * e + 128),
        clamp(c - 100 * d - 208 * e + 128),
        clamp(c + 516 * d + 128),
    ]
}

/// The other way: luma, then the two chroma values.
pub fn rgb_to_yuv([r, g, b]: [u8; 3]) -> [u8; 3] {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
    let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
    let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
    [y, u, v].map(|c| c.clamp(0, 255) as u8)
}

/// `img` scaled to cover `w` x `h` (what sticks out is cut off, as a wallpaper is), as I420.
pub fn cover_i420(img: &RgbFrame, w: usize, h: usize) -> I420Frame {
    let (iw, ih) = (img.width.max(1), img.height.max(1));
    let scale = f32::max(w as f32 / iw as f32, h as f32 / ih as f32);
    let (ox, oy) = (
        (iw as f32 * scale - w as f32) / 2.0,
        (ih as f32 * scale - h as f32) / 2.0,
    );
    let pick = |x: usize, y: usize| {
        let sx = (((x as f32 + 0.5 + ox) / scale) as usize).min(iw - 1);
        let sy = (((y as f32 + 0.5 + oy) / scale) as usize).min(ih - 1);
        let i = (sy * iw + sx) * 3;
        match img.data.get(i..i + 3) {
            Some(p) => [p[0], p[1], p[2]],
            None => [0, 0, 0],
        }
    };
    let mut data = vec![128u8; I420Frame::len_for(w, h)];
    let (luma, chroma) = data.split_at_mut(w * h);
    let (u, v) = chroma.split_at_mut(w * h / 4);
    for (y, line) in luma.chunks_exact_mut(w).enumerate() {
        for (x, l) in line.iter_mut().enumerate() {
            let [py, pu, pv] = rgb_to_yuv(pick(x, y));
            *l = py;
            if x % 2 == 0 && y % 2 == 0 {
                let c = (y / 2) * (w / 2) + x / 2;
                if c < u.len() {
                    u[c] = pu;
                    v[c] = pv;
                }
            }
        }
    }
    I420Frame {
        width: w,
        height: h,
        data,
    }
}

/// `frame` as the model's input: planar RGB in 0..1, `w` x `h`, the nearest pixel of each.
pub fn model_input(frame: &I420Frame, w: usize, h: usize) -> Vec<f32> {
    let (fy, fu, fv) = frame.planes();
    let chroma_w = frame.width / 2;
    let source = |i: usize, dst: usize, src: usize| {
        (((i as f32 + 0.5) * src as f32 / dst as f32) as usize).min(src - 1)
    };
    let xs: Vec<usize> = (0..w).map(|x| source(x, w, frame.width)).collect();
    let plane = w * h;
    let mut out = vec![0f32; 3 * plane];
    for y in 0..h {
        let sy = source(y, h, frame.height);
        for (x, &sx) in xs.iter().enumerate() {
            let ci = (sy / 2) * chroma_w + sx / 2;
            let rgb = yuv_to_rgb(fy[sy * frame.width + sx], fu[ci], fv[ci]);
            let i = y * w + x;
            for (c, value) in rgb.into_iter().enumerate() {
                out[c * plane + i] = f32::from(value) / 255.0;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warm_and_cool_filters_move_the_colour_and_keep_the_light() {
        let [y, u, v] = filter_luts(Filter::Warm).unwrap();
        assert_eq!(y[100], 100);
        assert!(u[128] < 128 && v[128] > 128, "less blue, more red");
        let [y, u, v] = filter_luts(Filter::Cool).unwrap();
        assert_eq!(y[100], 100);
        assert!(u[128] > 128 && v[128] < 128);
        assert_eq!(u[255], 255, "clamped");
        assert!(filter_luts(Filter::None).is_none());
    }

    #[test]
    fn black_and_white_drops_the_colour_and_vivid_adds_some() {
        let [_, u, v] = filter_luts(Filter::Mono).unwrap();
        assert!(u.iter().chain(&v).all(|&c| c == 128));
        let [y, u, _] = filter_luts(Filter::Vivid).unwrap();
        assert!(u[160] > 160 && u[96] < 96, "further from grey");
        assert_eq!(u[128], 128, "grey stays grey");
        assert!(y[200] > 200 && y[60] < 60, "more contrast");
    }

    #[test]
    fn the_low_light_boost_is_monotonic_and_keeps_black_and_white() {
        let lut = brighten_lut();
        assert_eq!(lut[16], 16);
        assert_eq!(lut[235], 235);
        assert_eq!(lut[5], 5);
        assert!(lut[60] > 60);
        assert!(lut.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn a_blur_keeps_a_flat_picture_and_spreads_a_dot() {
        let (w, h) = (9, 7);
        let flat = vec![90u8; w * h];
        let (mut out, mut tmp) = (Vec::new(), Vec::new());
        blur(&flat, w, h, 2, &mut out, &mut tmp);
        assert_eq!(out, flat);

        let mut dot = vec![0u8; w * h];
        dot[3 * w + 4] = 255;
        blur(&dot, w, h, 1, &mut out, &mut tmp);
        assert!(out[3 * w + 4] < 255 && out[3 * w + 4] > 0);
        assert!(out[3 * w + 5] > 0 && out[2 * w + 4] > 0, "spread around");
        assert_eq!(out[0], 0, "not far");
        blur(&dot, w, h, 0, &mut out, &mut tmp);
        assert_eq!(out, dot, "radius 0 copies");
    }

    #[test]
    fn compositing_takes_the_person_from_the_frame_and_the_rest_from_the_background() {
        let mut fg = vec![200u8, 200, 200];
        composite(&mut fg, &[10, 10, 10], &[255, 0, 128]);
        assert_eq!(fg[0], 200, "the person");
        assert_eq!(fg[1], 10, "the background");
        assert!((100..=110).contains(&fg[2]), "the edge mixes: {}", fg[2]);
    }

    #[test]
    fn blur_compositing_on_a_tiny_frame_blurs_only_the_background() {
        // 8x2: a sharp checker; the person is the left half.
        let (w, h) = (8, 2);
        let frame: Vec<u8> = (0..w * h)
            .map(|i| if i % 2 == 0 { 220 } else { 30 })
            .collect();
        let alpha: Vec<u8> = (0..w * h)
            .map(|i| if i % w < 4 { 255 } else { 0 })
            .collect();
        let (mut blurred, mut tmp) = (Vec::new(), Vec::new());
        blur(&frame, w, h, 1, &mut blurred, &mut tmp);
        let mut out = frame.clone();
        composite(&mut out, &blurred, &alpha);
        assert_eq!(out[..4], frame[..4], "the person stays sharp");
        let spread = |v: &[u8]| v.iter().max().unwrap() - v.iter().min().unwrap();
        assert!(spread(&out[4..8]) < spread(&frame[4..8]) / 2);
    }

    #[test]
    fn a_replacement_covers_the_frame_and_keeps_its_colour() {
        let red = RgbFrame {
            width: 4,
            height: 2,
            data: [220, 30, 30].repeat(8),
        };
        let bg = cover_i420(&red, 8, 4);
        assert_eq!((bg.width, bg.height), (8, 4));
        let (y, u, v) = bg.planes();
        let [ey, eu, ev] = rgb_to_yuv([220, 30, 30]);
        assert!(y.iter().all(|&s| s == ey));
        assert!(u.iter().all(|&s| s == eu) && v.iter().all(|&s| s == ev));

        // Replacing: where the mask says background the frame becomes the picture.
        let mut frame = I420Frame::new(8, 4, vec![128; I420Frame::len_for(8, 4)]).unwrap();
        let alpha = vec![0u8; 32];
        let mut alpha_c = Vec::new();
        half(&alpha, 8, 4, &mut alpha_c);
        assert_eq!(alpha_c.len(), 8);
        let (fy, fu, fv) = frame.planes_mut();
        composite(fy, y, &alpha);
        composite(fu, u, &alpha_c);
        composite(fv, v, &alpha_c);
        assert_eq!(frame, bg);
    }

    #[test]
    fn a_mask_scales_up_smoothly_and_keeps_solid_areas() {
        let m = Mask {
            width: 2,
            height: 1,
            data: vec![0, 255],
        };
        let mut out = Vec::new();
        upscale(&m, 8, 2, &mut out);
        assert_eq!(out.len(), 16);
        assert_eq!(out[0], 0);
        assert_eq!(out[7], 255);
        assert!(out[..8].windows(2).all(|w| w[0] <= w[1]), "{out:?}");
        assert!(out[3] > 0 && out[4] < 255, "a soft edge in the middle");
        assert_eq!(out[..8], out[8..]);
    }

    #[test]
    fn probabilities_become_a_mask_with_a_short_edge_and_smooth_over_time() {
        let m = Mask::from_probabilities(&[0.1, 0.5, 0.9, 1.0], 2, 2);
        assert_eq!(m.data, [0, 128, 255, 255]);
        let mut prev = Mask {
            width: 2,
            height: 2,
            data: vec![0, 0, 255, 255],
        };
        smooth(&mut prev, &m);
        assert_eq!(prev.data, [0, 96, 255, 255]);
    }

    #[test]
    fn rgb_and_yuv_round_trip_closely() {
        for rgb in [[0, 0, 0], [255, 255, 255], [200, 40, 90], [30, 140, 220]] {
            let [y, u, v] = rgb_to_yuv(rgb);
            let back = yuv_to_rgb(y, u, v);
            for (a, b) in rgb.iter().zip(back) {
                assert!(a.abs_diff(b) <= 4, "{rgb:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn the_model_sees_the_frame_as_rgb_in_0_to_1() {
        let [y, u, v] = rgb_to_yuv([255, 0, 0]);
        let mut data = vec![y; 16];
        data.extend([u; 4]);
        data.extend([v; 4]);
        let f = I420Frame::new(4, 4, data).unwrap();
        let input = model_input(&f, 2, 2);
        assert_eq!(input.len(), 12);
        assert!(input[..4].iter().all(|&r| r > 0.95), "red");
        assert!(input[4..].iter().all(|&c| c < 0.08), "no green or blue");
    }
}
