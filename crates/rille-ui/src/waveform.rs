//! Waveform views drawn in Rust: pixels are written straight into an image
//! buffer each frame and blitted once. Scrolling view (playhead centred, beat
//! grid, cues, loop) and track overview, in the style chosen in the settings;
//! optionally the scrolling view follows the channel's mixer controls.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qimage.h");
        type QImage = cxx_qt_lib::QImage;
        include!("cxx-qt-lib/qpainter.h");
        type QPainter = cxx_qt_lib::QPainter;
        include!("cxx-qt-lib/qrect.h");
        type QRect = cxx_qt_lib::QRect;
        include!("cxx-qt-lib/qsizef.h");
        type QSizeF = cxx_qt_lib::QSizeF;
    }

    unsafe extern "C++Qt" {
        include!(<QtQuick/QQuickPaintedItem>);
        #[qobject]
        type QQuickPaintedItem;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QQuickPaintedItem]
        #[qproperty(i32, deck)]
        #[qproperty(bool, overview)]
        /// Seconds of audio across the scrolling view.
        #[qproperty(f64, seconds)]
        type WaveformView = super::WaveformViewRust;

        #[cxx_override]
        unsafe fn paint(self: Pin<&mut WaveformView>, painter: *mut QPainter);

        /// Track time at horizontal position `x` (overview: absolute; scrolling: relative to the playhead).
        #[qinvokable]
        #[cxx_name = "timeAt"]
        fn time_at(self: &WaveformView, x: f64) -> f64;
    }

    unsafe extern "RustQt" {
        #[inherit]
        fn size(self: &WaveformView) -> QSizeF;
    }

    impl cxx_qt::Constructor<()> for WaveformView {}
}

use core::pin::Pin;

use cxx_qt::CxxQtType;

use cxx_qt_lib::{QImage, QImageFormat, QRect};
use rille_app::{Settings, WaveformStyle};
use rille_core::{BeatClock, BeatGrid, WaveformSummary};
use rille_engine::ChannelState;

use crate::global::{app, playhead};

pub struct WaveformViewRust {
    deck: i32,
    overview: bool,
    seconds: f64,
    pixels: Vec<u32>,
}

impl Default for WaveformViewRust {
    fn default() -> Self {
        Self { deck: 0, overview: false, seconds: 8.0, pixels: Vec::new() }
    }
}

impl cxx_qt::Constructor<()> for qobject::WaveformView {
    type BaseArguments = ();
    type NewArguments = ();
    type InitializeArguments = ();

    fn route_arguments(_: ()) -> ((), (), ()) {
        ((), (), ())
    }

    fn new(_: ()) -> WaveformViewRust {
        WaveformViewRust::default()
    }
}

// Matches Theme.bg / Theme.accent (Signal) in Theme.qml.
const BG: u32 = 0x0a0a09;
const ENVELOPE: u32 = 0x2c2b28;
const LOOP: u32 = 0x2ec4b6;
const CUE: u32 = 0xff4a1c;
const PLAYHEAD: u32 = 0xf4f3ef;
const END_WARNING: u32 = 0xec3f5c;
const THREE_BAND: [u32; 3] = [0x2a6cf0, 0xe8942c, 0xf4f4f0];
const MONO: u32 = 0x2f7fe8;

struct Canvas<'a> {
    w: usize,
    h: usize,
    px: &'a mut [u32],
    /// Bars grow up from the bottom edge instead of around the centre.
    bottom: bool,
}

fn blend(dst: u32, src: u32, a: f32) -> u32 {
    let ch = |shift: u32| {
        let d = ((dst >> shift) & 0xff) as f32;
        let s = ((src >> shift) & 0xff) as f32;
        ((d + (s - d) * a).round() as u32).min(255) << shift
    };
    0xff00_0000 | ch(16) | ch(8) | ch(0)
}

impl Canvas<'_> {
    fn vspan(&mut self, x: usize, y0: usize, y1: usize, color: u32, alpha: f32) {
        if x >= self.w {
            return;
        }
        for y in y0.min(self.h)..y1.min(self.h) {
            let p = &mut self.px[y * self.w + x];
            *p = if alpha >= 1.0 { 0xff00_0000 | color } else { blend(*p, color, alpha) };
        }
    }

    /// A line that stays visible on any waveform: darkens bright pixels and
    /// lightens dark ones by `strength`.
    fn vline_contrast(&mut self, x: f64, strength: f32, width: usize) {
        if x < -1.0 || x > self.w as f64 {
            return;
        }
        let x0 = x.round().max(0.0) as usize;
        for dx in 0..width {
            let xx = x0 + dx;
            if xx >= self.w {
                continue;
            }
            for y in 0..self.h {
                let p = &mut self.px[y * self.w + xx];
                let target = if luma(*p) > 0.45 { 0x000000 } else { 0xffffff };
                *p = blend(*p, target, strength);
            }
        }
    }

    /// Small triangle pointing into the waveform from the top or bottom edge.
    fn marker(&mut self, x: f64, top: bool, size: usize, color: u32) {
        for k in 0..size {
            let half = (size - k) as f64 / 2.0;
            let y = if top { k } else { self.h.saturating_sub(1 + k) };
            let (a, b) = ((x - half).round().max(0.0) as usize, (x + half).round().max(0.0) as usize);
            for xx in a..=b.min(self.w.saturating_sub(1)) {
                if y < self.h {
                    self.px[y * self.w + xx] = 0xff00_0000 | color;
                }
            }
        }
    }

    fn vline(&mut self, x: f64, color: u32, alpha: f32, width: usize) {
        if x < -1.0 || x > self.w as f64 {
            return;
        }
        let x0 = x.round().max(0.0) as usize;
        for dx in 0..width {
            self.vspan(x0 + dx, 0, self.h, color, alpha);
        }
    }

    fn rect(&mut self, x0: f64, x1: f64, y0: usize, y1: usize, color: u32, alpha: f32) {
        let a = x0.max(0.0).round() as usize;
        let b = (x1.min(self.w as f64).round().max(0.0) as usize).min(self.w);
        for x in a..b {
            self.vspan(x, y0, y1, color, alpha);
        }
    }

    /// Bar for `v` in 0..1: mirrored around the vertical centre, or up from
    /// the bottom edge.
    fn bar(&mut self, x: usize, v: f32, color: u32, alpha: f32) {
        let v = v.clamp(0.0, 1.0) * 0.96;
        if self.bottom {
            let e = (v * self.h as f32).round() as usize;
            self.vspan(x, self.h.saturating_sub(e.max(1)), self.h, color, alpha);
        } else {
            let e = (v * self.h as f32 / 2.0).round() as usize;
            let c = self.h / 2;
            self.vspan(x, c.saturating_sub(e), c + e.max(1), color, alpha);
        }
    }
}

/// Max of each band over bins `[a, b)`.
fn bins_max(wf: &WaveformSummary, a: f64, b: f64) -> [u8; 4] {
    let n = wf.bins.len() as i64;
    let (i0, i1) = (a.floor() as i64, (b.ceil() as i64).max(a.floor() as i64 + 1));
    let mut m = [0u8; 4];
    for i in i0.max(0)..i1.min(n) {
        let v = wf.bins[i as usize];
        for k in 0..4 {
            m[k] = m[k].max(v[k]);
        }
    }
    m
}

fn luma(p: u32) -> f32 {
    let ch = |s: u32| ((p >> s) & 0xff) as f32 / 255.0;
    0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
}

/// Column color from its band levels (0..1): bass red, mids green, highs
/// blue, mixed like light, so kicks glow warm, hats cool and full-range
/// parts near white. Never darker than a readable minimum.
pub fn spectral(low: f32, mid: f32, high: f32) -> u32 {
    let (l, m, h) = (low.max(0.0).powf(1.5), (mid.max(0.0) * 1.15).powf(1.5), (high.max(0.0) * 1.7).powf(1.5));
    let max = l.max(m).max(h);
    if max <= 1e-6 {
        return 0x484641;
    }
    let floor = 0.22;
    let ch = |v: f32| ((floor + (1.0 - floor) * (v / max)) * 255.0).round() as u32;
    (ch(l) << 16) | (ch(m) << 8) | ch(h)
}

/// Saturated column color: bass red, mids green, highs blue, scaled so the
/// strongest band is at full brightness.
pub fn rgb_mix(low: f32, mid: f32, high: f32) -> u32 {
    let (r, g, b) = (low.max(0.0), mid.max(0.0) * 0.8, high.max(0.0) * 1.4);
    let max = r.max(g).max(b);
    if max <= 1e-6 {
        return 0x33322e;
    }
    let ch = |v: f32| ((v / max).powf(2.5) * 255.0).round() as u32;
    (ch(r) << 16) | (ch(g) << 8) | ch(b)
}

/// How the columns of one view are drawn.
#[derive(Clone, Copy)]
struct Look {
    style: WaveformStyle,
    /// Vertical scale.
    height: f32,
    /// Scale of the low, mid and high band as drawn (1 = as analyzed).
    bands: [f32; 3],
    alpha: f32,
}

impl Look {
    fn new(s: &Settings) -> Self {
        Self { style: s.waveform_style, height: s.waveform_height.clamp(0.5, 2.0), bands: [1.0; 3], alpha: 1.0 }
    }

    /// Applies the channel's mixer controls, as far as the settings ask for it.
    fn with_mixer(mut self, s: &Settings, ch: &ChannelState, crossfader_gain: f32) -> Self {
        if s.waveform_mixer {
            // Bins store sqrt(peak), so an amplitude gain g scales them by sqrt(g).
            self.bands = rille_engine::band_gains(ch).map(|g| g.max(0.0).sqrt());
        }
        if s.waveform_fader_dim {
            // Dim rather than shrink: a faded-out deck is still being cued.
            self.alpha = 0.3 + 0.7 * rille_engine::fader_level(ch, crossfader_gain).sqrt();
        }
        self
    }
}

fn draw_column(c: &mut Canvas, x: usize, m: [u8; 4], look: &Look) {
    let f = |v: u8| f32::from(v) / 255.0 * look.height;
    let [low, mid, high] = [0, 1, 2].map(|k| f(m[k]) * look.bands[k]);
    let a = look.alpha;
    // The envelope as analyzed: it stays behind as an outline where the
    // mixer takes bands out.
    c.bar(x, f(m[3]), ENVELOPE, a);
    match look.style {
        WaveformStyle::Spectrum => {
            let color = spectral(low, mid, high);
            // Body: the loudest band's extent, in the column's color; core:
            // the high band, lighter, so hats and claps read as texture.
            c.bar(x, low.max(mid * 0.85).max(high * 0.6), color, 0.95 * a);
            c.bar(x, high * 0.45, blend(color, 0xffffff, 0.45) & 0xff_ffff, 0.9 * a);
        }
        WaveformStyle::ThreeBand => {
            for (k, v) in [low, mid * 0.75, high * 0.4].into_iter().enumerate() {
                c.bar(x, v, THREE_BAND[k], 0.95 * a);
            }
        }
        WaveformStyle::Rgb => c.bar(x, low.max(mid * 0.85).max(high * 0.6), rgb_mix(low, mid, high), 0.95 * a),
        WaveformStyle::Mono => {
            let body = low.max(mid * 0.85).max(high * 0.6);
            let bright = if body > 1e-3 { (high * 0.6 / body).min(1.0) } else { 0.0 };
            c.bar(x, body, blend(MONO, 0xffffff, 0.1 + 0.6 * bright) & 0xff_ffff, 0.95 * a);
        }
    }
}

impl qobject::WaveformView {
    unsafe fn paint(mut self: Pin<&mut Self>, painter: *mut cxx_qt_lib::QPainter) {
        let size = self.size();
        let (w, h) = (size.width().max(1.0) as usize, size.height().max(1.0) as usize);
        let (deck, overview, seconds) = (*self.deck(), *self.overview(), *self.seconds());
        let mut rust = self.as_mut().rust_mut();
        let buf = &mut rust.pixels;
        buf.clear();
        buf.resize(w * h, 0xff00_0000 | BG);
        let settings = app().map(|a| a.settings()).unwrap_or_default();
        let mut canvas = Canvas { w, h, px: buf.as_mut_slice(), bottom: settings.waveform_bottom };
        if let Some(app) = app() {
            let d = deck.clamp(0, 3) as usize;
            let snap = app.snapshot();
            let look = Look::new(&settings);
            let info = app.deck(d as u8);
            let s = snap.decks[d];
            let pos = playhead(d, &snap);
            let cues: Vec<(f64, u32)> = s
                .hotcues
                .iter()
                .enumerate()
                .filter_map(|(i, h)| h.map(|h| (h.secs, rille_core::track::HOTCUE_COLORS[i])))
                .collect();
            if overview {
                draw_overview(
                    &mut canvas,
                    look,
                    info.waveform.as_deref(),
                    pos,
                    s.duration_secs,
                    s.main_cue_secs,
                    &cues,
                    s.loop_set.then_some((s.loop_start_secs, s.loop_end_secs, s.loop_active)),
                );
            } else {
                draw_scrolling(
                    &mut canvas,
                    look.with_mixer(&settings, &snap.channels[d], snap.crossfader_gain(d)),
                    info.waveform.as_deref(),
                    info.grid.as_deref(),
                    pos,
                    seconds.max(1.0),
                    s.main_cue_secs,
                    s.loaded,
                    &cues,
                    s.loop_set.then_some((s.loop_start_secs, s.loop_end_secs, s.loop_active)),
                );
            }
        }
        extern "C" fn keep(_: *mut cxx_qt_lib::c_void) {}
        // SAFETY: the buffer outlives the image, which is drawn and dropped
        // before this function returns.
        let image = unsafe {
            QImage::from_raw_parts(
                buf.as_ptr().cast::<u8>(),
                w as i32,
                h as i32,
                QImageFormat::Format_RGB32,
                keep,
                std::ptr::null_mut(),
            )
        };
        if let Some(p) = unsafe { painter.as_mut() } {
            let p = unsafe { Pin::new_unchecked(p) };
            p.draw_image(&QRect::new(0, 0, w as i32, h as i32), &image);
        }
        drop(image);
    }

    fn time_at(&self, x: f64) -> f64 {
        let Some(app) = app() else { return 0.0 };
        let d = (*self.deck()).clamp(0, 3) as usize;
        let s = app.snapshot().decks[d];
        let w = self.size().width().max(1.0);
        if *self.overview() {
            (x / w).clamp(0.0, 1.0) * s.duration_secs
        } else {
            s.position_secs + (x - w / 2.0) * *self.seconds() / w
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_scrolling(
    c: &mut Canvas,
    look: Look,
    wf: Option<&WaveformSummary>,
    grid: Option<&BeatGrid>,
    pos: f64,
    seconds: f64,
    main_cue: f64,
    loaded: bool,
    cues: &[(f64, u32)],
    loop_: Option<(f64, f64, bool)>,
) {
    let (w, h) = (c.w as f64, c.h);
    let spp = seconds / w;
    let x_of = |t: f64| w / 2.0 + (t - pos) / spp;
    if let Some((a, b, active)) = loop_ {
        c.rect(x_of(a), x_of(b), 0, h, LOOP, if active { 0.22 } else { 0.08 });
    }
    if let Some(wf) = wf {
        let bps = wf.bins_per_sec;
        for x in 0..c.w {
            let t0 = pos + (x as f64 - w / 2.0) * spp;
            if t0 < 0.0 {
                continue;
            }
            draw_column(c, x, bins_max(wf, t0 * bps, (t0 + spp) * bps), &look);
        }
    }
    if let Some(g) = grid {
        let (b0, b1) = (g.beat_at(pos - seconds / 2.0).floor() as i64, g.beat_at(pos + seconds / 2.0).ceil() as i64);
        // Skip beat lines when too dense to read.
        let beat_px = 60.0 / g.bpm_at(pos).max(1.0) / spp;
        for b in b0..=b1 {
            let x = x_of(g.secs_at(b as f64));
            if g.is_downbeat(b) {
                c.vline_contrast(x, 0.6, 2);
                c.marker(x + 0.5, true, 7, 0xf2c94c);
                c.marker(x + 0.5, false, 7, 0xf2c94c);
            } else if beat_px > 6.0 {
                c.vline_contrast(x, 0.35, 1);
            }
        }
    }
    if loaded {
        let x = x_of(main_cue);
        c.vline(x, CUE, 1.0, 2);
        c.rect(x - 5.0, x + 7.0, 0, 6, CUE, 1.0);
    }
    for &(t, color) in cues {
        let x = x_of(t);
        c.vline(x, color, 0.9, 2);
        c.rect(x, x + 10.0, 0, 10, color, 1.0);
    }
    c.vline(w / 2.0 - 1.0, PLAYHEAD, 1.0, 2);
}

#[allow(clippy::too_many_arguments)]
fn draw_overview(
    c: &mut Canvas,
    look: Look,
    wf: Option<&WaveformSummary>,
    pos: f64,
    duration: f64,
    main_cue: f64,
    cues: &[(f64, u32)],
    loop_: Option<(f64, f64, bool)>,
) {
    let w = c.w as f64;
    if duration <= 0.0 {
        return;
    }
    let x_of = |t: f64| t / duration * w;
    if let Some(wf) = wf {
        let n = wf.bins.len() as f64;
        for x in 0..c.w {
            let (a, b) = (x as f64 / w * n, (x as f64 + 1.0) / w * n);
            let played = x_of(pos) > x as f64;
            let look = Look { alpha: if played { 0.45 } else { 1.0 }, ..look };
            draw_column(c, x, bins_max(wf, a, b), &look);
        }
    }
    if let Some((a, b, active)) = loop_ {
        c.rect(x_of(a), x_of(b).max(x_of(a) + 2.0), 0, c.h, LOOP, if active { 0.35 } else { 0.15 });
    }
    c.vline(x_of(main_cue), CUE, 1.0, 1);
    for &(t, color) in cues {
        c.vline(x_of(t), color, 1.0, 2);
    }
    c.vline(x_of(pos), PLAYHEAD, 1.0, 2);
    // Last 30 seconds marked.
    let warn = x_of((duration - 30.0).max(0.0));
    c.rect(warn, w, c.h - 3, c.h, END_WARNING, 0.6);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectral_colors() {
        let rgb = |c: u32| ((c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff);
        // A kick is red, a hat blue, everything at once near white.
        let (r, g, b) = rgb(spectral(1.0, 0.1, 0.05));
        assert!(r > 200 && g < 100 && b < 100, "kick {r} {g} {b}");
        let (r, g, b) = rgb(spectral(0.05, 0.1, 0.7));
        assert!(b > 200 && r < 100, "hat {r} {g} {b}");
        let (r, g, b) = rgb(spectral(0.9, 0.8, 0.55));
        assert!(r > 180 && g > 180 && b > 180, "full {r} {g} {b}");
        // Silence is a dim grey, not black.
        assert_eq!(spectral(0.0, 0.0, 0.0), 0x484641);
        // RGB: saturated, the dominant band at full brightness.
        let (r, g, b) = rgb(rgb_mix(1.0, 0.2, 0.1));
        assert!(r == 255 && g < 40 && b < 40, "kick {r} {g} {b}");
    }

    fn render(style: WaveformStyle, bottom: bool, bands: [f32; 3]) -> Vec<u32> {
        let (w, h) = (4, 40);
        let mut px = vec![BG; w * h];
        let mut c = Canvas { w, h, px: &mut px, bottom };
        let look = Look { style, height: 1.0, bands, alpha: 1.0 };
        for x in 0..w {
            draw_column(&mut c, x, [220, 120, 60, 230], &look);
        }
        px
    }

    fn lit_rows(px: &[u32], w: usize) -> Vec<usize> {
        (0..px.len() / w).filter(|&y| px[y * w] & 0xff_ffff != BG).collect()
    }

    #[test]
    fn styles_shapes_and_mixer() {
        for style in WaveformStyle::ALL {
            let mirrored = lit_rows(&render(style, false, [1.0; 3]), 4);
            assert!(mirrored.contains(&20) && mirrored.contains(&5), "{style:?} centred");
            let bottom = lit_rows(&render(style, true, [1.0; 3]), 4);
            assert!(bottom.contains(&39) && !bottom.contains(&0), "{style:?} from the bottom");
            // A killed band changes the picture but leaves the outline.
            let killed = render(style, false, [0.0, 1.0, 1.0]);
            assert_ne!(killed, render(style, false, [1.0; 3]), "{style:?}");
            assert_eq!(lit_rows(&killed, 4), mirrored, "{style:?} outline stays");
        }
    }
}
