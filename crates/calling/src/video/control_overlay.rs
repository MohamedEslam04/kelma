//! The controller's own cursor over a share under remote control, as Teams draws it: an arrow
//! with their name (and photo) that moves while the presenter's real pointer stays free
//! ([`crate::remote_control::PointerPolicy`] decides what reaches the real pointer).
//!
//! It is a wlr-layer-shell surface (KWin, sway, Hyprland implement it; GNOME does not) on the
//! overlay layer, with an empty input region (click-through) and no keyboard focus, drawn in
//! software into a shm buffer. It runs on its own thread with its own Wayland connection. The
//! surface is only as large as the arrow and label; moving it is a margin change and a commit.
//! Without layer-shell (or Wayland) there is no overlay, logged, and control works as before.
//!
//! Limits:
//! - a monitor share covers the output the portal names by position (else by size, else the
//!   first one); a window share is only known by its size, not where the window is, so its area
//!   is centered on that output and the drawn cursor is off by the window's offset from center;
//! - the overlay is above every window, so it shows in the shared picture too (the controller
//!   sees their cursor drawn where their pointer is);
//! - the label is laid out left to right without shaping (Arabic names show unjoined).
//!
//! Windows and macOS have no overlay yet (nor remote control, `super::remote_input`): only the
//! geometry and the drawing are built there, for the tests.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[cfg(target_os = "linux")]
use std::sync::Arc;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
#[cfg(target_os = "linux")]
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
#[cfg(target_os = "linux")]
use smithay_client_toolkit::output::{OutputHandler, OutputState};
#[cfg(target_os = "linux")]
use smithay_client_toolkit::reexports::calloop::EventLoop;
#[cfg(target_os = "linux")]
use smithay_client_toolkit::reexports::calloop::channel::{self, Channel, Sender};
#[cfg(target_os = "linux")]
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
#[cfg(target_os = "linux")]
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
#[cfg(target_os = "linux")]
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_shm, wl_surface};
#[cfg(target_os = "linux")]
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
#[cfg(target_os = "linux")]
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
#[cfg(target_os = "linux")]
use smithay_client_toolkit::shell::WaylandSurface;
#[cfg(target_os = "linux")]
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
#[cfg(target_os = "linux")]
use smithay_client_toolkit::shm::slot::SlotPool;
#[cfg(target_os = "linux")]
use smithay_client_toolkit::shm::{Shm, ShmHandler};
#[cfg(target_os = "linux")]
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    registry_handlers,
};

use crate::remote_control::Cursor;

/// What the share covers, as the portal described its stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShareArea {
    /// A whole monitor (else a window, or unknown).
    pub monitor: bool,
    /// The stream's position in the compositor's logical space (monitors only, on KDE).
    pub position: Option<(i32, i32)>,
    /// The stream's size.
    pub size: (u32, u32),
}

/// An output, in the compositor's logical space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputGeom {
    pub position: (i32, i32),
    pub size: (i32, i32),
}

/// A rectangle in an output's surface-local (logical) coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The output the shared area is on, and the area within it.
pub fn place(outputs: &[OutputGeom], area: &ShareArea) -> Option<(usize, Rect)> {
    let size = (
        i32::try_from(area.size.0).unwrap_or(i32::MAX),
        i32::try_from(area.size.1).unwrap_or(i32::MAX),
    );
    let contains = |o: &OutputGeom, (x, y): (i32, i32)| {
        x >= o.position.0
            && y >= o.position.1
            && x < o.position.0 + o.size.0
            && y < o.position.1 + o.size.1
    };
    let index = area
        .position
        .and_then(|p| outputs.iter().position(|o| contains(o, p)))
        .or_else(|| {
            area.monitor
                .then(|| outputs.iter().position(|o| o.size == size))
                .flatten()
        })
        .or((!outputs.is_empty()).then_some(0))?;
    let o = outputs[index];
    if area.monitor {
        return Some((
            index,
            Rect {
                x: 0,
                y: 0,
                w: o.size.0,
                h: o.size.1,
            },
        ));
    }
    let (w, h) = (
        size.0.clamp(1, o.size.0.max(1)),
        size.1.clamp(1, o.size.1.max(1)),
    );
    let (x, y) = match area.position {
        Some(p) => (p.0 - o.position.0, p.1 - o.position.1),
        None => ((o.size.0 - w) / 2, (o.size.1 - h) / 2),
    };
    Some((index, Rect { x, y, w, h }))
}

/// The surface's top-left margin that puts the arrow's tip (`hotspot`, in the surface) on the
/// normalized point of `area`. `None` off the area.
pub fn cursor_margin(area: Rect, x: f64, y: f64, hotspot: (i32, i32)) -> Option<(i32, i32)> {
    if !crate::remote_control::on_picture(x, y) {
        return None;
    }
    let at = |v: f64, start: i32, len: i32| {
        start + ((v * f64::from(len)).round() as i32).min((len - 1).max(0))
    };
    Some((
        (at(y, area.y, area.h) - hotspot.1).max(0),
        (at(x, area.x, area.w) - hotspot.0).max(0),
    ))
}

/// The longest name shown; longer ones end in an ellipsis.
const MAX_LABEL_CHARS: usize = 24;

/// The name as the label shows it.
pub fn label_text(name: &str) -> String {
    let name = name.trim();
    if name.chars().count() <= MAX_LABEL_CHARS {
        return name.to_owned();
    }
    let mut s: String = name.chars().take(MAX_LABEL_CHARS - 1).collect();
    s.truncate(s.trim_end().len());
    s.push('…');
    s
}

/// The arrow's outline, tip first, in logical pixels from the tip.
const ARROW: [(f32, f32); 7] = [
    (0.0, 0.0),
    (0.0, 17.0),
    (4.2, 13.2),
    (7.0, 19.5),
    (9.6, 18.4),
    (6.9, 12.2),
    (12.4, 12.2),
];
/// Where the tip is in the surface (room for the arrow's white edge).
const TIP: (i32, i32) = (2, 2);
const PILL_H: i32 = 24;
const PILL_AT: (i32, i32) = (14, 20);
const PHOTO: i32 = 18;
const PAD: i32 = 9;
const TEXT_PX: f32 = 13.0;

/// Where everything is, in logical pixels of the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub size: (i32, i32),
    /// The arrow's tip.
    pub hotspot: (i32, i32),
    pub pill: Rect,
    pub photo: Option<Rect>,
    /// The left end of the text and the vertical middle of the pill.
    pub text_at: (i32, i32),
}

/// The layout for a label `text_width` logical pixels wide, with a photo or not.
pub fn layout(text_width: i32, photo: bool) -> Layout {
    let (px, py) = PILL_AT;
    let photo = photo.then_some(Rect {
        x: px + (PILL_H - PHOTO) / 2,
        y: py + (PILL_H - PHOTO) / 2,
        w: PHOTO,
        h: PHOTO,
    });
    let text_x = photo.map_or(px + PAD, |r| r.x + r.w + 6);
    let pill = Rect {
        x: px,
        y: py,
        w: (text_x - px) + text_width.max(0) + PAD,
        h: PILL_H,
    };
    Layout {
        size: (pill.x + pill.w, pill.y + pill.h),
        hotspot: TIP,
        pill,
        photo,
        text_at: (text_x, py + PILL_H / 2),
    }
}

/// A profile photo: straight (not premultiplied) RGBA, `width * height * 4` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Photo {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Teams' cursor colour (its brand purple).
const ACCENT: [u8; 3] = [0x62, 0x64, 0xa7];
const WHITE: [u8; 3] = [0xff; 3];

/// A software canvas: premultiplied ARGB, as `wl_shm`'s `Argb8888` wants it (little endian
/// B, G, R, A bytes).
struct Canvas {
    w: i32,
    h: i32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(w: i32, h: i32) -> Self {
        Canvas {
            w,
            h,
            px: vec![0; (w.max(0) * h.max(0) * 4) as usize],
        }
    }

    /// Paints `rgb` over a pixel at coverage `a` (0..=1).
    fn blend(&mut self, x: i32, y: i32, rgb: [u8; 3], a: f32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let i = ((y * self.w + x) * 4) as usize;
        let p = &mut self.px[i..i + 4];
        let src = [rgb[2], rgb[1], rgb[0], 255];
        for (d, s) in p.iter_mut().zip(src) {
            *d = (f32::from(s) * a + f32::from(*d) * (1.0 - a)).round() as u8;
        }
    }
}

fn inside(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut c = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let ((xi, yi), (xj, yj)) = (poly[i], poly[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            c = !c;
        }
        j = i;
    }
    c
}

fn edge_distance(poly: &[(f32, f32)], x: f32, y: f32) -> f32 {
    let mut best = f32::MAX;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let ((ax, ay), (bx, by)) = (poly[j], poly[i]);
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
        let (ex, ey) = (ax + t * dx - x, ay + t * dy - y);
        best = best.min((ex * ex + ey * ey).sqrt());
        j = i;
    }
    best
}

/// Draws the arrow at `scale`: the accent colour inside a white edge, 4x4 supersampled.
fn draw_arrow(c: &mut Canvas, scale: f32) {
    let poly: Vec<(f32, f32)> = ARROW
        .iter()
        .map(|&(x, y)| ((x + TIP.0 as f32) * scale, (y + TIP.1 as f32) * scale))
        .collect();
    let edge = 1.4 * scale;
    let (w, h) = (
        ((14.4 + TIP.0 as f32 * 2.0) * scale) as i32,
        ((19.5 + TIP.1 as f32 * 2.0) * scale) as i32 + 1,
    );
    for py in 0..h {
        for px in 0..w {
            let (mut fill, mut white) = (0u32, 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let (x, y) = (
                        px as f32 + (sx as f32 + 0.5) / 4.0,
                        py as f32 + (sy as f32 + 0.5) / 4.0,
                    );
                    let d = edge_distance(&poly, x, y);
                    if inside(&poly, x, y) {
                        if d >= edge * 0.6 {
                            fill += 1;
                        } else {
                            white += 1;
                        }
                    } else if d < edge * 0.6 {
                        white += 1;
                    }
                }
            }
            c.blend(px, py, WHITE, white as f32 / 16.0);
            c.blend(px, py, ACCENT, fill as f32 / 16.0);
        }
    }
}

/// Coverage (0..=1) of a pixel by a rounded rectangle of radius `r`, 4x4 supersampled.
fn rounded_cover(r: (f32, f32, f32, f32), radius: f32, px: i32, py: i32) -> f32 {
    let (x0, y0, x1, y1) = r;
    let mut n = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            let (x, y) = (
                px as f32 + (sx as f32 + 0.5) / 4.0,
                py as f32 + (sy as f32 + 0.5) / 4.0,
            );
            if x < x0 || x > x1 || y < y0 || y > y1 {
                continue;
            }
            let cx = x.clamp(x0 + radius, x1 - radius);
            let cy = y.clamp(y0 + radius, y1 - radius);
            if (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius {
                n += 1;
            }
        }
    }
    n as f32 / 16.0
}

fn scaled(r: Rect, s: f32) -> (f32, f32, f32, f32) {
    (
        r.x as f32 * s,
        r.y as f32 * s,
        (r.x + r.w) as f32 * s,
        (r.y + r.h) as f32 * s,
    )
}

/// Draws a photo into a circle: center-cropped to a square, nearest sample.
fn draw_photo(c: &mut Canvas, r: Rect, s: f32, photo: &Photo) {
    let (x0, y0, x1, y1) = scaled(r, s);
    let side = photo.width.min(photo.height).max(1) as f32;
    let (ox, oy) = (
        (photo.width as f32 - side) / 2.0,
        (photo.height as f32 - side) / 2.0,
    );
    let radius = (x1 - x0) / 2.0;
    for py in y0 as i32..y1.ceil() as i32 {
        for px in x0 as i32..x1.ceil() as i32 {
            let cover = rounded_cover((x0, y0, x1, y1), radius, px, py);
            if cover <= 0.0 {
                continue;
            }
            let u = ((px as f32 + 0.5 - x0) / (x1 - x0) * side + ox) as u32;
            let v = ((py as f32 + 0.5 - y0) / (y1 - y0) * side + oy) as u32;
            let i = ((v.min(photo.height - 1) * photo.width + u.min(photo.width - 1)) * 4) as usize;
            let Some(p) = photo.rgba.get(i..i + 4) else {
                continue;
            };
            c.blend(px, py, [p[0], p[1], p[2]], cover * f32::from(p[3]) / 255.0);
        }
    }
}

/// The width of `text` at `TEXT_PX`, in logical pixels.
fn text_width(font: &FontVec, text: &str) -> f32 {
    let f = font.as_scaled(PxScale::from(TEXT_PX));
    let mut w = 0.0;
    let mut last = None;
    for ch in text.chars() {
        let id = f.glyph_id(ch);
        if let Some(prev) = last {
            w += f.kern(prev, id);
        }
        w += f.h_advance(id);
        last = Some(id);
    }
    w
}

fn draw_text(c: &mut Canvas, font: &FontVec, text: &str, at: (i32, i32), s: f32) {
    let f = font.as_scaled(PxScale::from(TEXT_PX * s));
    let baseline = at.1 as f32 * s + (f.ascent() + f.descent()) / 2.0;
    let mut x = at.0 as f32 * s;
    let mut last = None;
    for ch in text.chars() {
        let id = f.glyph_id(ch);
        if let Some(prev) = last {
            x += f.kern(prev, id);
        }
        let glyph = id.with_scale_and_position(f.scale(), point(x, baseline));
        x += f.h_advance(id);
        last = Some(id);
        if let Some(g) = font.outline_glyph(glyph) {
            let b = g.px_bounds();
            g.draw(|gx, gy, cov| {
                c.blend(
                    b.min.x as i32 + gx as i32,
                    b.min.y as i32 + gy as i32,
                    WHITE,
                    cov,
                );
            });
        }
    }
}

/// The overlay's picture at `scale` (buffer pixels per logical pixel).
fn render(
    l: &Layout,
    scale: i32,
    font: Option<&FontVec>,
    text: &str,
    photo: Option<&Photo>,
) -> Canvas {
    let s = scale as f32;
    let mut c = Canvas::new(l.size.0 * scale, l.size.1 * scale);
    let pill = scaled(l.pill, s);
    let radius = l.pill.h as f32 * s / 2.0;
    for py in pill.1 as i32..pill.3.ceil() as i32 {
        for px in pill.0 as i32..pill.2.ceil() as i32 {
            let cover = rounded_cover(pill, radius, px, py);
            c.blend(px, py, ACCENT, cover * 0.94);
        }
    }
    if let (Some(r), Some(p)) = (l.photo, photo) {
        draw_photo(&mut c, r, s, p);
    }
    if let Some(font) = font {
        draw_text(&mut c, font, text, l.text_at, s);
    }
    draw_arrow(&mut c, s);
    c
}

/// A sans-serif font from fontconfig (else a few usual paths). `None`: the label has no text.
fn load_font() -> Option<FontVec> {
    let matched = std::process::Command::new("fc-match")
        .args(["--format=%{file}", "sans-serif:bold"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok());
    let candidates = matched.into_iter().chain(
        [
            "/usr/share/fonts/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/google-noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        ]
        .map(str::to_owned),
    );
    for path in candidates {
        if let Ok(bytes) = std::fs::read(&path)
            && let Ok(font) = FontVec::try_from_vec(bytes)
        {
            return Some(font);
        }
    }
    tracing::info!("no font for the control cursor's label");
    None
}

/// The controller's cursor on screen. Dropping it removes the overlay.
pub struct ControlOverlay {
    #[cfg(target_os = "linux")]
    tx: Sender<Cursor>,
}

#[cfg(not(target_os = "linux"))]
impl ControlOverlay {
    /// No overlay on this system: nothing is drawn.
    pub fn show(_area: ShareArea, _name: &str, _photo: Option<Photo>) -> ControlOverlay {
        tracing::info!("no control cursor overlay on this system");
        ControlOverlay {}
    }

    pub fn cursor(&self, _c: Cursor) {}
}

#[cfg(target_os = "linux")]
impl ControlOverlay {
    /// Shows `name`'s cursor (hidden until the first point) over `area`, on its own thread.
    /// Failing (no Wayland, no layer-shell) only logs.
    pub fn show(area: ShareArea, name: &str, photo: Option<Photo>) -> ControlOverlay {
        let (tx, rx) = channel::channel();
        let label = label_text(name);
        let spawned = std::thread::Builder::new()
            .name("control-cursor".into())
            .spawn(move || {
                if let Err(e) = run(area, label, photo.map(Arc::new), rx) {
                    tracing::info!(%e, "no control cursor overlay");
                }
            });
        if let Err(e) = spawned {
            tracing::warn!(%e, "control cursor thread not started");
        }
        ControlOverlay { tx }
    }

    pub fn cursor(&self, c: Cursor) {
        let _ = self.tx.send(c);
    }
}

#[cfg(target_os = "linux")]
struct State {
    registry: RegistryState,
    outputs: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: Option<LayerSurface>,
    /// The picture, ARGB at the buffer scale.
    image: Canvas,
    area: Rect,
    hotspot: (i32, i32),
    configured: bool,
    visible: bool,
    pending: Option<Cursor>,
    exit: bool,
}

#[cfg(target_os = "linux")]
impl State {
    /// Attaches the picture (or a clear one) and commits.
    fn attach(&mut self, visible: bool) {
        let Some(layer) = &self.layer else { return };
        let (w, h) = (self.image.w, self.image.h);
        match self
            .pool
            .create_buffer(w, h, w * 4, wl_shm::Format::Argb8888)
        {
            Ok((buffer, canvas)) => {
                if visible {
                    canvas.copy_from_slice(&self.image.px);
                } else {
                    canvas.fill(0);
                }
                layer.wl_surface().damage_buffer(0, 0, w, h);
                if let Err(e) = buffer.attach_to(layer.wl_surface()) {
                    tracing::debug!(?e, "control cursor buffer not attached");
                    return;
                }
                self.visible = visible;
            }
            Err(e) => tracing::debug!(%e, "control cursor buffer not created"),
        }
    }

    /// Carries out the latest cursor change.
    fn apply(&mut self) {
        if !self.configured {
            return;
        }
        let Some(c) = self.pending.take() else { return };
        let at = match c {
            Cursor::At(x, y) => cursor_margin(self.area, x, y, self.hotspot),
            Cursor::Hidden => None,
        };
        match at {
            Some((top, left)) => {
                if let Some(layer) = &self.layer {
                    layer.set_margin(top, 0, 0, left);
                }
                if !self.visible {
                    self.attach(true);
                }
            }
            None if self.visible => self.attach(false),
            None => return,
        }
        if let Some(layer) = &self.layer {
            layer.commit();
        }
    }
}

#[cfg(target_os = "linux")]
fn run(
    area: ShareArea,
    label: String,
    photo: Option<Arc<Photo>>,
    rx: Channel<Cursor>,
) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland display: {e}"))?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&conn).map_err(|e| format!("Wayland registry: {e}"))?;
    let qh = queue.handle();
    let compositor =
        CompositorState::bind(&globals, &qh).map_err(|e| format!("wl_compositor: {e}"))?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).map_err(|e| format!("no wlr-layer-shell: {e}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm: {e}"))?;
    let pool = SlotPool::new(64 * 1024, &shm).map_err(|e| format!("shm pool: {e}"))?;
    let mut state = State {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer: None,
        image: Canvas::new(0, 0),
        area: Rect::default(),
        hotspot: TIP,
        configured: false,
        visible: false,
        pending: None,
        exit: false,
    };
    // Outputs, then their xdg-output logical geometry.
    for _ in 0..2 {
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("Wayland: {e}"))?;
    }
    let outputs: Vec<_> = state
        .outputs
        .outputs()
        .filter_map(|o| state.outputs.info(&o).map(|i| (o, i)))
        .collect();
    let geoms: Vec<OutputGeom> = outputs
        .iter()
        .map(|(_, i)| OutputGeom {
            position: i.logical_position.unwrap_or(i.location),
            size: i.logical_size.unwrap_or_else(|| {
                i.modes
                    .iter()
                    .find(|m| m.current)
                    .map_or((0, 0), |m| m.dimensions)
            }),
        })
        .collect();
    let (index, rect) = place(&geoms, &area).ok_or("no output")?;
    let (output, info) = &outputs[index];
    let scale = info.scale_factor.max(1);

    let font = load_font();
    let width = font.as_ref().map_or(0.0, |f| text_width(f, &label)).ceil() as i32;
    let l = layout(width, photo.is_some());
    state.image = render(&l, scale, font.as_ref(), &label, photo.as_deref());
    state.area = rect;
    state.hotspot = l.hotspot;

    let surface = compositor.create_surface(&qh);
    surface.set_buffer_scale(scale);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("kelma-control-cursor"),
        Some(output),
    );
    layer.set_anchor(Anchor::TOP | Anchor::LEFT);
    layer.set_size(l.size.0 as u32, l.size.1 as u32);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    // Margins from the output's edge, not from panels' exclusive zones.
    layer.set_exclusive_zone(-1);
    let region = Region::new(&compositor).map_err(|e| format!("wl_region: {e}"))?;
    layer.set_input_region(Some(region.wl_region()));
    layer.commit();
    state.layer = Some(layer);
    tracing::info!(?rect, scale, "control cursor overlay shown");

    let mut event_loop: EventLoop<State> =
        EventLoop::try_new().map_err(|e| format!("event loop: {e}"))?;
    WaylandSource::new(conn.clone(), queue)
        .insert(event_loop.handle())
        .map_err(|e| format!("event loop: {e}"))?;
    event_loop
        .handle()
        .insert_source(rx, |ev, _, st: &mut State| match ev {
            channel::Event::Msg(c) => st.pending = Some(c),
            channel::Event::Closed => st.exit = true,
        })
        .map_err(|e| format!("event loop: {e}"))?;
    while !state.exit {
        event_loop
            .dispatch(None, &mut state)
            .map_err(|e| format!("event loop: {e}"))?;
        state.apply();
        let _ = conn.flush();
    }
    drop(state.layer.take());
    let _ = conn.flush();
    Ok(())
}

#[cfg(target_os = "linux")]
impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

#[cfg(target_os = "linux")]
impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

#[cfg(target_os = "linux")]
impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        if !self.configured {
            self.configured = true;
            // Mapped clear, so the first point only moves and shows it.
            self.attach(false);
            if let Some(layer) = &self.layer {
                layer.commit();
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

#[cfg(target_os = "linux")]
impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

#[cfg(target_os = "linux")]
delegate_compositor!(State);
#[cfg(target_os = "linux")]
delegate_output!(State);
#[cfg(target_os = "linux")]
delegate_shm!(State);
#[cfg(target_os = "linux")]
delegate_layer!(State);
#[cfg(target_os = "linux")]
delegate_registry!(State);

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: OutputGeom = OutputGeom {
        position: (0, 0),
        size: (1920, 1080),
    };
    const RIGHT: OutputGeom = OutputGeom {
        position: (1920, 0),
        size: (2560, 1440),
    };

    #[test]
    fn a_monitor_share_covers_the_output_the_portal_names() {
        let both = [LAPTOP, RIGHT];
        let by_position = ShareArea {
            monitor: true,
            position: Some((1920, 0)),
            size: (5120, 2880),
        };
        let whole = Rect {
            x: 0,
            y: 0,
            w: 2560,
            h: 1440,
        };
        assert_eq!(place(&both, &by_position), Some((1, whole)));
        // No position: the output of that size, else the first.
        let by_size = ShareArea {
            monitor: true,
            position: None,
            size: (2560, 1440),
        };
        assert_eq!(place(&both, &by_size), Some((1, whole)));
        let unknown = ShareArea {
            size: (800, 600),
            ..by_size
        };
        assert_eq!(place(&both, &unknown).map(|p| p.0), Some(0));
        assert_eq!(place(&[], &unknown), None);
    }

    #[test]
    fn a_window_share_is_centered_when_its_position_is_unknown() {
        let window = ShareArea {
            monitor: false,
            position: None,
            size: (1000, 600),
        };
        assert_eq!(
            place(&[LAPTOP], &window),
            Some((
                0,
                Rect {
                    x: 460,
                    y: 240,
                    w: 1000,
                    h: 600
                }
            ))
        );
        // Larger than the output: clamped to it.
        let huge = ShareArea {
            size: (4000, 3000),
            ..window
        };
        assert_eq!(
            place(&[LAPTOP], &huge).map(|p| p.1),
            Some(Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080
            })
        );
        let known = ShareArea {
            position: Some((2000, 100)),
            ..window
        };
        assert_eq!(
            place(&[LAPTOP, RIGHT], &known),
            Some((
                1,
                Rect {
                    x: 80,
                    y: 100,
                    w: 1000,
                    h: 600
                }
            ))
        );
    }

    #[test]
    fn a_normalized_point_maps_onto_the_area_less_the_hotspot() {
        let area = Rect {
            x: 460,
            y: 240,
            w: 1000,
            h: 600,
        };
        assert_eq!(cursor_margin(area, 0.5, 0.5, (2, 2)), Some((538, 958)));
        assert_eq!(cursor_margin(area, 1.0, 1.0, (0, 0)), Some((839, 1459)));
        let corner = Rect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        };
        assert_eq!(cursor_margin(corner, 0.0, 0.0, (2, 2)), Some((0, 0)));
        assert_eq!(cursor_margin(area, 1.01, 0.5, (2, 2)), None);
        assert_eq!(cursor_margin(area, 0.5, f64::NAN, (2, 2)), None);
    }

    #[test]
    fn the_label_holds_the_photo_and_the_text() {
        let l = layout(80, true);
        let photo = l.photo.unwrap();
        assert_eq!(l.hotspot, TIP);
        assert!(photo.x > l.pill.x && photo.y >= l.pill.y);
        assert!(photo.y + photo.h <= l.pill.y + l.pill.h);
        assert!(l.text_at.0 >= photo.x + photo.w);
        assert_eq!(l.text_at.0 + 80 + PAD, l.pill.x + l.pill.w);
        assert_eq!(l.size, (l.pill.x + l.pill.w, l.pill.y + l.pill.h));
        // The label starts below and right of the arrow's tip.
        assert!(l.pill.x > l.hotspot.0 && l.pill.y > l.hotspot.1);

        let plain = layout(80, false);
        assert_eq!(plain.photo, None);
        assert_eq!(plain.text_at.0, plain.pill.x + PAD);
        assert!(plain.size.0 < l.size.0);
        assert_eq!(layout(-5, false).pill.w, 2 * PAD);
    }

    #[test]
    fn long_names_end_in_an_ellipsis() {
        assert_eq!(label_text("  Ada Lovelace "), "Ada Lovelace");
        let long = "Augusta Ada King, Countess of Lovelace";
        let t = label_text(long);
        assert!(t.ends_with('…') && t.chars().count() <= MAX_LABEL_CHARS);
        assert_eq!(label_text(&"ب".repeat(24)), "ب".repeat(24));
    }

    #[test]
    fn the_picture_has_the_arrow_at_its_tip_and_is_clear_elsewhere() {
        let l = layout(40, true);
        let photo = Photo {
            width: 2,
            height: 2,
            rgba: [255, 0, 0, 255].repeat(4),
        };
        let c = render(&l, 2, None, "", Some(&photo));
        assert_eq!((c.w, c.h), (l.size.0 * 2, l.size.1 * 2));
        let alpha = |x: i32, y: i32| c.px[((y * c.w + x) * 4 + 3) as usize];
        // Inside the arrow, a little below the tip: opaque.
        assert_eq!(alpha((TIP.0 + 3) * 2, (TIP.1 + 8) * 2), 255);
        // Top-right corner, beside the arrow and above the label: clear.
        assert_eq!(alpha(c.w - 1, 0), 0);
        // The photo's middle is red.
        let p = l.photo.unwrap();
        let (x, y) = ((p.x + p.w / 2) * 2, (p.y + p.h / 2) * 2);
        let i = ((y * c.w + x) * 4) as usize;
        assert_eq!(&c.px[i..i + 4], &[0, 0, 255, 255]);
    }
}
