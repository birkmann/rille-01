//! One deck: transport (play, CUE/CUP, hotcues, loops, beatjump, flux,
//! reverse, scratch) and rendering through two voices, so jumps and loop
//! wraps are crossfaded instead of clicking.

use std::sync::Arc;

use rille_core::quantize::phase_preserving_target;
use rille_core::{BeatClock, BeatGrid, Control, ControlValue, CueKind};
use rille_dsp::SincTable;

use crate::mixer::fader_gain;
use crate::remix::RemixDeck;
use crate::types::{
    DEFAULT_LOOP_SIZE, Event, Garbage, HOTCUES, Hotcue, LOOP_SIZES, LoadedTrack, STEMS, StemAudio, TrackAudio,
};
use crate::voice::{READ_REACH, StemMix, Voice};

pub(crate) struct RenderCtx<'a> {
    pub sr_out: f64,
    pub sinc: &'a SincTable,
    /// Crossfade length for jumps, output frames.
    pub declick: usize,
}

/// Global switches that change how deck buttons behave.
#[derive(Clone, Copy)]
pub(crate) struct Modes {
    pub quantize: bool,
    pub snap: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Held {
    None,
    /// CUE held while previewing from the cue point.
    Cue,
    Cup,
    Hotcue(u8),
}

#[derive(Clone, Copy, Debug)]
struct Roll {
    beats: f64,
    /// Where playback would be without the roll.
    slip: f64,
    /// The loop the roll replaced, restored when it ends.
    prev_loop: (Option<(f64, f64)>, bool),
}

/// FILTER ROLL length in beats for a filter knob position: none within 10 %
/// of the centre, then 1 beat, halving towards the ends down to 1/16.
pub fn filter_roll_beats(knob: f32) -> Option<f64> {
    let d = f64::from((knob - 0.5).abs() * 2.0);
    if d.is_nan() || d < 0.1 {
        return None;
    }
    let step = ((d - 0.1) / 0.9 * 5.0).floor().min(4.0) as i32;
    Some(0.5f64.powi(step))
}

/// Seconds of audio per jog wheel revolution (a 33⅓ rpm record).
const SECS_PER_REV: f64 = 1.8;
/// Pitch bend while a bend button is held.
const BEND: f64 = 0.03;
/// Audio a streaming deck keeps ahead of the play position.
const STREAM_READ_AHEAD_SECS: f64 = 0.5;
/// Fastest playback (source frames per output frame) that still plays the
/// stems mix; faster scratching reads the track.
const STEM_MAX_SPEED: usize = 8;

pub(crate) struct Deck {
    pub index: u8,
    pub track: Option<Arc<TrackAudio>>,
    pub grid: Option<Arc<BeatGrid>>,
    pub track_id: u64,
    /// Set when this is a remix deck (then there is no track).
    pub remix: Option<Box<RemixDeck>>,
    voices: [Voice; 2],
    active: usize,
    fade_pos: usize,
    fade_len: usize,
    /// Gain the fading-out voice starts from (below 1 when a jump came
    /// during a crossfade).
    fade_from: f32,
    pub buf: Vec<[f32; 2]>,
    fade_buf: Vec<[f32; 2]>,
    pub playing: bool,
    pub main_cue: f64,
    pub hotcues: [Option<Hotcue>; HOTCUES],
    held: Held,
    /// PLAY was pressed during a CUE/hotcue preview: keep playing on release.
    latched: bool,
    pub loop_range: Option<(f64, f64)>,
    pub loop_active: bool,
    pub loop_size: usize,
    loop_in: Option<f64>,
    pub flux: bool,
    /// Where playback would be without the current loop/reverse/hotcue hold.
    flux_pos: Option<f64>,
    /// Loop roll in progress, see [`Deck::roll`].
    roll: Option<Roll>,
    pub reverse: bool,
    /// Metronome on the grid beats (see `Control::Tick`).
    pub tick: bool,
    /// Click being played: samples since its start and whether it is an accent.
    tick_click: Option<(u32, bool)>,
    pub keylock: bool,
    /// Semitones, −12..12.
    pub key_shift: f32,
    pub tempo_fader: f32,
    /// −1, 0 or +1 while a bend button is held.
    pub bend: f64,
    pub sync: bool,
    /// Follower beats per master beat (½, 1 or 2).
    pub sync_mult: f64,
    /// Offset from the master's phase while a bend button is held (beats);
    /// glides back when released.
    pub phase_offset: f64,
    /// Offset the DJ set by nudging a synced deck (jog, touch strip): kept
    /// until sync is switched off and on or another track is loaded.
    pub jog_offset: f64,
    /// A jog nudge is still being applied (sync pulls it in quickly and
    /// without jumps).
    pub nudging: bool,
    jog_touch: bool,
    jog_accum: f64,
    scratch_speed: f64,
    /// Pending nudge (seconds) from turning the jog wheel's rim.
    nudge_secs: f64,
    pub auto_gain_db: f32,
    /// Track seconds per output second in the last block.
    pub speed: f64,
    ended_sent: bool,
    last_stretch: bool,
    /// Set when playback starts, so the engine can align a synced deck.
    pub just_started: bool,
    /// Seconds since sync last jumped this deck into phase.
    pub since_realign: f64,
    /// Jumps into phase since the track was loaded (diagnostics).
    pub realigns: u32,
    /// The whole track's length while it still arrives (see
    /// [`Command::ExtendTrack`](crate::Command::ExtendTrack)).
    streaming: Option<f64>,
    /// Playing, but at the end of what has arrived.
    pub buffering: bool,
    /// The track's stems, see [`StemAudio`].
    pub stems: Option<Arc<StemAudio>>,
    pub stem_volume: [f32; STEMS],
    pub stem_mute: [bool; STEMS],
    /// Stem gains now, gliding to their targets block by block.
    stem_gain: [f32; STEMS],
    /// Stems mixed for the frames a block reads.
    stem_scratch: Vec<[f32; 2]>,
}

impl Deck {
    pub fn new(index: u8, sample_rate: u32, max_block: usize) -> Self {
        Self {
            index,
            track: None,
            grid: None,
            track_id: 0,
            remix: None,
            voices: [Voice::new(sample_rate, max_block), Voice::new(sample_rate, max_block)],
            active: 0,
            fade_pos: 0,
            fade_len: 0,
            fade_from: 1.0,
            buf: vec![[0.0; 2]; max_block],
            fade_buf: vec![[0.0; 2]; max_block],
            playing: false,
            main_cue: 0.0,
            hotcues: [None; HOTCUES],
            held: Held::None,
            latched: false,
            loop_range: None,
            loop_active: false,
            loop_size: DEFAULT_LOOP_SIZE,
            loop_in: None,
            flux: false,
            flux_pos: None,
            roll: None,
            reverse: false,
            tick: false,
            tick_click: None,
            keylock: true,
            key_shift: 0.0,
            tempo_fader: 0.5,
            bend: 0.0,
            sync: false,
            sync_mult: 1.0,
            phase_offset: 0.0,
            jog_offset: 0.0,
            nudging: false,
            jog_touch: false,
            jog_accum: 0.0,
            scratch_speed: 0.0,
            nudge_secs: 0.0,
            auto_gain_db: 0.0,
            speed: 0.0,
            ended_sent: false,
            last_stretch: false,
            just_started: false,
            since_realign: 0.0,
            realigns: 0,
            streaming: None,
            buffering: false,
            stems: None,
            stem_volume: [1.0; STEMS],
            stem_mute: [false; STEMS],
            stem_gain: [1.0; STEMS],
            stem_scratch: vec![[0.0; 2]; STEM_MAX_SPEED * max_block + 2 * READ_REACH + 4],
        }
    }

    pub fn load(&mut self, t: LoadedTrack, garbage: &mut dyn FnMut(Garbage)) {
        if self.remix.is_some() {
            // Remix decks take samples into cells, not tracks.
            garbage(Garbage::Audio(t.audio));
            if let Some(g) = t.grid {
                garbage(Garbage::Grid(g));
            }
            return;
        }
        self.unload(garbage);
        self.track = Some(t.audio);
        self.grid = t.grid;
        self.track_id = t.id;
        self.realigns = 0;
        self.jog_offset = 0.0;
        self.nudging = false;
        self.main_cue = t.main_cue_secs;
        self.hotcues = t.hotcues;
        self.auto_gain_db = t.auto_gain_db;
        self.set_position(t.main_cue_secs);
    }

    pub fn unload(&mut self, garbage: &mut dyn FnMut(Garbage)) {
        if let Some(a) = self.track.take() {
            garbage(Garbage::Audio(a));
        }
        if let Some(g) = self.grid.take() {
            garbage(Garbage::Grid(g));
        }
        self.track_id = 0;
        self.playing = false;
        self.held = Held::None;
        self.latched = false;
        self.loop_range = None;
        self.loop_active = false;
        self.loop_in = None;
        self.flux_pos = None;
        self.roll = None;
        self.reverse = false;
        self.hotcues = [None; HOTCUES];
        self.main_cue = 0.0;
        self.ended_sent = false;
        self.fade_len = 0;
        self.streaming = None;
        self.buffering = false;
        if let Some(s) = self.stems.take() {
            garbage(Garbage::Stems(s));
        }
        self.stem_volume = [1.0; STEMS];
        self.stem_mute = [false; STEMS];
        self.stem_gain = [1.0; STEMS];
    }

    /// Stems for the track (see [`Command::SetStems`](crate::Command::SetStems)).
    pub fn set_stems(&mut self, track_id: u64, stems: Option<Arc<StemAudio>>, garbage: &mut dyn FnMut(Garbage)) {
        let fits = |s: &StemAudio| {
            self.remix.is_none()
                && self.track_id == track_id
                && self.streaming.is_none()
                && self
                    .track
                    .as_ref()
                    .is_some_and(|t| t.sample_rate == s.sample_rate && t.frames.len() == s.frames.len())
        };
        match stems {
            Some(s) if !fits(&s) => garbage(Garbage::Stems(s)),
            stems => {
                if self.track_id == track_id
                    && let Some(old) = std::mem::replace(&mut self.stems, stems)
                {
                    garbage(Garbage::Stems(old));
                }
            }
        }
    }

    /// Stem gains the controls ask for.
    fn stem_targets(&self) -> [f32; STEMS] {
        std::array::from_fn(|i| if self.stem_mute[i] { 0.0 } else { fader_gain(self.stem_volume[i]) })
    }

    /// Moves the stem gains towards their targets over `n` frames (a time
    /// constant of 15 ms, so faders and mutes don't click).
    fn glide_stems(&mut self, n: usize, sr_out: f64) {
        let k = 1.0 - (-(n as f64) / (0.015 * sr_out)).exp() as f32;
        let targets = self.stem_targets();
        for (g, t) in self.stem_gain.iter_mut().zip(targets) {
            *g += (t - *g) * k;
            if (t - *g).abs() < 1e-4 {
                *g = t;
            }
        }
    }

    /// Swaps in a longer copy of the track while it streams, see
    /// [`Command::ExtendTrack`](crate::Command::ExtendTrack).
    pub fn extend(
        &mut self,
        track_id: u64,
        audio: Arc<TrackAudio>,
        length_secs: Option<f64>,
        garbage: &mut dyn FnMut(Garbage),
    ) {
        let fits = self.remix.is_none()
            && self.track_id == track_id
            && self.track.as_ref().is_some_and(|t| t.sample_rate == audio.sample_rate);
        if !fits {
            garbage(Garbage::Audio(audio));
            return;
        }
        if let Some(old) = self.track.replace(audio) {
            garbage(Garbage::Audio(old));
        }
        self.streaming = length_secs;
    }

    /// Seconds of the track that have arrived.
    pub fn arrived_secs(&self) -> f64 {
        self.track.as_ref().map_or(0.0, |t| t.duration_secs())
    }

    /// Makes this a remix deck (`Some`) or an empty track deck (`None`).
    pub fn set_remix(&mut self, remix: Option<Box<RemixDeck>>, garbage: &mut dyn FnMut(Garbage)) {
        if let Some(old) = self.remix.take() {
            garbage(Garbage::Remix(old));
        }
        self.unload(garbage);
        if let Some(mut r) = remix {
            self.grid = r.grid.take();
            self.track_id = r.id;
            self.remix = Some(r);
        }
    }

    pub fn set_grid(&mut self, grid: Option<Arc<BeatGrid>>, garbage: &mut dyn FnMut(Garbage)) {
        // A remix deck keeps its beat when its tempo changes.
        if let (Some(r), Some(old), Some(new)) = (self.remix.as_mut(), &self.grid, &grid) {
            r.pos = new.secs_at(old.beat_at(r.pos));
        }
        if let Some(old) = std::mem::replace(&mut self.grid, grid) {
            garbage(Garbage::Grid(old));
        }
    }

    fn sr(&self) -> f64 {
        self.track.as_ref().map_or(44_100.0, |t| f64::from(t.sample_rate))
    }

    /// The whole track's length, also while it still arrives.
    pub fn duration(&self) -> f64 {
        self.arrived_secs().max(self.streaming.unwrap_or(0.0))
    }

    /// Position being heard now, seconds.
    pub fn position(&self) -> f64 {
        match &self.remix {
            Some(r) => r.pos,
            None => self.voices[self.active].audible() / self.sr(),
        }
    }

    pub fn beat(&self) -> Option<f64> {
        self.grid.as_ref().map(|g| g.beat_at(self.position()))
    }

    /// Track tempo at the play position.
    pub fn track_bpm(&self) -> Option<f64> {
        self.grid.as_ref().map(|g| g.bpm_at(self.position()))
    }

    /// Own playback rate from the tempo fader and bend buttons.
    pub fn own_rate(&self, tempo_range: f64) -> f64 {
        (1.0 + (f64::from(self.tempo_fader) - 0.5) * 2.0 * tempo_range) * (1.0 + self.bend * BEND)
    }

    pub fn is_scratching(&self) -> bool {
        self.jog_touch
    }

    pub fn cue_held(&self) -> bool {
        matches!(self.held, Held::Cue | Held::Cup)
    }

    fn uses_stretch(&self) -> bool {
        (self.keylock || self.key_shift != 0.0) && !self.reverse && !self.jog_touch
    }

    /// Jump without a crossfade (nothing is playing, or during load).
    pub fn set_position(&mut self, secs: f64) {
        self.place(secs, self.uses_stretch());
    }

    /// Moves a paused deck while it is scrubbed (jog wheel, touch strip):
    /// without priming the time-stretcher, which costs about a millisecond
    /// per move with keylock on. Nothing plays through it while paused, and
    /// a voice primes itself when it starts playing (see `Voice::render`).
    fn scrub_to(&mut self, secs: f64) {
        self.place(secs, false);
    }

    fn place(&mut self, secs: f64, stretch: bool) {
        if let Some(r) = &mut self.remix {
            // The slots follow on the next block.
            r.pos = secs;
            return;
        }
        let Some(track) = self.track.clone() else { return };
        let sr = self.sr();
        let (voice, speed) = (self.active, self.speed.max(0.01));
        self.voice_jump(voice, &track, secs * sr, speed, stretch);
        self.fade_len = 0;
        self.ended_sent = false;
    }

    /// Voice `v` jumps to source frame `target`, reading the track with its
    /// stems at their current gains.
    fn voice_jump(&mut self, v: usize, track: &TrackAudio, target: f64, speed: f64, stretch: bool) {
        let gains = self.stem_gain;
        match self.stems.as_deref().filter(|_| gains != [1.0; STEMS]) {
            Some(stems) => {
                let mut mix = StemMix { track, stems, gains, scratch: &mut self.stem_scratch };
                self.voices[v].jump(&mut mix, target, speed, stretch);
            }
            None => self.voices[v].jump(&mut &*track, target, speed, stretch),
        }
    }

    /// Jump to `secs`, crossfading from the current sound if playing.
    pub fn jump(&mut self, secs: f64, ctx: &RenderCtx) {
        if self.remix.is_some() {
            // Slots crossfade to their new positions by themselves.
            self.set_position(secs);
            return;
        }
        let Some(track) = self.track.clone() else { return };
        if !self.playing || ctx.declick == 0 {
            self.set_position(secs);
            return;
        }
        let sr = self.sr();
        let speed_frames = (self.speed * sr / ctx.sr_out).max(0.01);
        // During a crossfade (a loop wrap right after a jump into phase) the
        // louder voice fades out from where it is and the quieter one is
        // reused, instead of cutting off the voice that is still fading out.
        let (keep, gain) = match self.fade_gains() {
            Some((gi, go)) if go > gi => (1 - self.active, go),
            Some((gi, _)) => (self.active, gi),
            None => (self.active, 1.0),
        };
        let other = 1 - keep;
        let stretch = self.uses_stretch();
        self.voice_jump(other, &track, secs * sr, speed_frames, stretch);
        self.active = other;
        self.fade_pos = 0;
        self.fade_len = ctx.declick;
        self.fade_from = gain;
        self.ended_sent = false;
    }

    /// Whether a jump's crossfade is still running.
    pub fn fading(&self) -> bool {
        self.fade_pos < self.fade_len
    }

    /// Gains of the incoming and outgoing voice at the current fade position.
    fn fade_gains(&self) -> Option<(f32, f32)> {
        self.fading().then(|| {
            let g = self.fade_pos as f32 / self.fade_len as f32 * std::f32::consts::FRAC_PI_2;
            (g.sin(), g.cos() * self.fade_from)
        })
    }

    /// Target of a jump: with quantize, the position near `secs` that keeps
    /// the current offset within the beat, so a synced deck stays in phase.
    fn quantized(&self, secs: f64, m: &Modes) -> f64 {
        match &self.grid {
            Some(g) if m.quantize && self.playing => {
                g.secs_at(phase_preserving_target(g.beat_at(self.position()), g.beat_at(secs)))
            }
            _ => secs,
        }
    }

    /// With snap, the nearest beat to `secs`.
    fn snapped(&self, secs: f64, m: &Modes) -> f64 {
        match &self.grid {
            Some(g) if m.snap => g.secs_at(g.beat_at(secs).round()),
            _ => secs,
        }
    }

    fn start_flux(&mut self) {
        if self.flux && self.playing && self.flux_pos.is_none() {
            self.flux_pos = Some(self.position());
        }
    }

    fn end_flux(&mut self, ctx: &RenderCtx) {
        if let Some(p) = self.flux_pos.take() {
            self.jump(p, ctx);
        }
    }

    fn set_playing(&mut self, on: bool) {
        if on && !self.playing {
            self.just_started = true;
        }
        self.playing = on && (self.track.is_some() || self.remix.is_some());
    }

    /// Auto loop from the current beat, `LOOP_SIZES[loop_size]` beats long.
    fn auto_loop(&mut self, ctx: &RenderCtx) {
        let size = LOOP_SIZES[self.loop_size];
        let pos = self.position();
        let (start, end) = match &self.grid {
            Some(g) => {
                let b = g.beat_at(pos);
                let unit = size.min(1.0);
                let sb = (b / unit + 1e-6).floor() * unit;
                (g.secs_at(sb), g.secs_at(sb + size))
            }
            None => (pos, pos + size * 0.5),
        };
        self.start_flux();
        self.loop_range = Some((start, end));
        self.loop_active = true;
        let _ = ctx;
    }

    /// Loop roll of `beats` around the current beat while `Some`, back to
    /// normal playback on `None`. A roll always slips: it ends where the
    /// track would be, inside a loop that was active before it. Needs a
    /// playing deck with a grid; changing the length re-cuts the loop around
    /// the playhead.
    pub fn roll(&mut self, beats: Option<f64>, ctx: &RenderCtx) {
        match beats {
            Some(size) if self.playing && size > 0.0 => {
                let Some(g) = &self.grid else { return };
                if self.roll.is_some_and(|r| r.beats == size) {
                    return;
                }
                let pos = self.position();
                let b = (g.beat_at(pos) / size + 1e-6).floor() * size;
                let range = (g.secs_at(b), g.secs_at(b + size));
                let prev_loop = (self.loop_range, self.loop_active);
                let r = self.roll.get_or_insert(Roll { beats: size, slip: pos, prev_loop });
                r.beats = size;
                self.loop_range = Some(range);
                self.loop_active = true;
            }
            Some(_) => {}
            None => {
                if let Some(r) = self.roll.take() {
                    (self.loop_range, self.loop_active) = r.prev_loop;
                    self.jump(r.slip, ctx);
                }
            }
        }
    }

    fn resize_loop(&mut self) {
        if let (Some((s, _)), Some(g)) = (self.loop_range, &self.grid) {
            let e = g.secs_at(g.beat_at(s) + LOOP_SIZES[self.loop_size]);
            self.loop_range = Some((s, e));
        }
    }

    fn beatjump(&mut self, dir: f64, ctx: &RenderCtx) {
        let size = LOOP_SIZES[self.loop_size] * dir;
        let pos = self.position();
        let target = match &self.grid {
            Some(g) => g.secs_at(g.beat_at(pos) + size),
            None => pos + size * 0.5,
        };
        let delta = target - pos;
        if self.loop_active
            && let Some((s, e)) = self.loop_range
        {
            self.loop_range = Some((s + delta, e + delta));
        }
        self.jump(target.max(0.0), ctx);
    }

    /// Transport controls. Returns an event to report, if any.
    pub fn control(&mut self, c: Control, v: ControlValue, m: &Modes, ctx: &RenderCtx) -> Option<Event> {
        let press = matches!(v, ControlValue::Press(true));
        let release = matches!(v, ControlValue::Press(false));
        let (deck, track_id) = (self.index, self.track_id);
        if self.remix.is_some() && self.remix_control(c, v, ctx) {
            return None;
        }
        if self.track.is_none()
            && !matches!(
                c,
                Control::Tempo
                    | Control::TempoReset
                    | Control::Keylock
                    | Control::Tick
                    | Control::Sync
                    | Control::KeyShift
                    | Control::Flux
            )
        {
            return None;
        }
        match c {
            Control::StemVolume(n @ 1..=4) => {
                if let ControlValue::Absolute(x) = v {
                    self.stem_volume[usize::from(n - 1)] = x.clamp(0.0, 1.0);
                }
            }
            Control::StemMute(n @ 1..=4) if press => {
                let m = &mut self.stem_mute[usize::from(n - 1)];
                *m = !*m;
            }
            Control::Play if press => {
                if matches!(self.held, Held::Cue | Held::Hotcue(_)) {
                    self.latched = true;
                } else {
                    self.set_playing(!self.playing);
                }
            }
            Control::Cue if press => {
                let pos = self.position();
                if self.playing {
                    self.playing = false;
                    self.set_position(self.main_cue);
                } else {
                    let mut event = None;
                    if (pos - self.main_cue).abs() > 0.005 {
                        self.main_cue = self.snapped(pos, m);
                        self.set_position(self.main_cue);
                        event = Some(Event::MainCueSet { deck, track_id, secs: self.main_cue });
                    }
                    self.held = Held::Cue;
                    self.latched = false;
                    self.set_playing(true);
                    return event;
                }
            }
            Control::Cue if release && self.held == Held::Cue => {
                self.held = Held::None;
                if !self.latched {
                    self.playing = false;
                    self.set_position(self.main_cue);
                }
            }
            Control::Cup if press => {
                self.playing = false;
                self.set_position(self.main_cue);
                self.held = Held::Cup;
            }
            Control::Cup if release && self.held == Held::Cup => {
                self.held = Held::None;
                self.set_playing(true);
            }
            Control::Hotcue(n) if (1..=HOTCUES as u8).contains(&n) => {
                let slot = n - 1;
                if press {
                    match self.hotcues[usize::from(slot)] {
                        None => {
                            let (kind, secs, len_secs) = match (self.loop_active, self.loop_range) {
                                (true, Some((s, e))) => (CueKind::Loop, s, e - s),
                                _ => (CueKind::Cue, self.snapped(self.position(), m), 0.0),
                            };
                            let cue = Hotcue { secs, kind, len_secs };
                            self.hotcues[usize::from(slot)] = Some(cue);
                            return Some(Event::HotcueSet { deck, track_id, slot, cue });
                        }
                        Some(cue) => {
                            if cue.kind == CueKind::Loop && cue.len_secs > 0.0 {
                                self.loop_range = Some((cue.secs, cue.secs + cue.len_secs));
                                self.loop_active = true;
                            }
                            if self.playing {
                                self.start_flux();
                                if self.flux_pos.is_some() {
                                    self.held = Held::Hotcue(slot);
                                }
                                let target = self.quantized(cue.secs, m);
                                self.jump(target, ctx);
                            } else {
                                self.set_position(cue.secs);
                                self.held = Held::Hotcue(slot);
                                self.latched = false;
                                self.set_playing(true);
                            }
                        }
                    }
                } else if release && self.held == Held::Hotcue(slot) {
                    self.held = Held::None;
                    if self.flux_pos.is_some() && !self.loop_active {
                        self.end_flux(ctx);
                    } else if !self.latched && self.flux_pos.is_none() {
                        // Preview from pause: stop and return to the hotcue.
                        self.playing = false;
                        if let Some(cue) = self.hotcues[usize::from(slot)] {
                            self.set_position(cue.secs);
                        }
                    }
                }
            }
            Control::HotcueDelete(n) if press && (1..=HOTCUES as u8).contains(&n) => {
                let slot = n - 1;
                if self.hotcues[usize::from(slot)].take().is_some() {
                    return Some(Event::HotcueDeleted { deck, track_id, slot });
                }
            }
            Control::LoopToggle if press => {
                if self.loop_active {
                    self.loop_active = false;
                    self.end_flux(ctx);
                } else {
                    self.auto_loop(ctx);
                }
            }
            Control::LoopIn if press => {
                let p = self.snapped(self.position(), m);
                self.loop_in = Some(p);
                if self.loop_active
                    && let Some((_, e)) = self.loop_range
                    && p < e
                {
                    self.loop_range = Some((p, e));
                }
            }
            Control::LoopOut if press => {
                let p = self.snapped(self.position(), m);
                match (self.loop_in, self.loop_range) {
                    (Some(s), _) if p > s + 0.01 => {
                        self.start_flux();
                        self.loop_range = Some((s, p));
                        self.loop_active = true;
                        self.loop_in = None;
                    }
                    (_, Some(_)) if !self.loop_active => {
                        self.start_flux();
                        self.loop_active = true;
                    }
                    _ => {}
                }
            }
            Control::LoopSizeDown | Control::LoopHalve if press => {
                self.loop_size = self.loop_size.saturating_sub(1);
                if self.loop_active {
                    self.resize_loop();
                }
            }
            Control::LoopSizeUp | Control::LoopDouble if press => {
                self.loop_size = (self.loop_size + 1).min(LOOP_SIZES.len() - 1);
                if self.loop_active {
                    self.resize_loop();
                }
            }
            Control::BeatjumpBack if press => self.beatjump(-1.0, ctx),
            Control::BeatjumpForward if press => self.beatjump(1.0, ctx),
            Control::JumpStart if press => {
                // A loop further on would pull the playhead back into it,
                // and flux would return to where the track would be.
                self.loop_active = false;
                self.flux_pos = None;
                self.jump(0.0, ctx);
            }
            Control::Flux if press => {
                self.flux = !self.flux;
                if !self.flux {
                    self.flux_pos = None;
                }
            }
            Control::Reverse => {
                if press && !self.reverse {
                    self.start_flux();
                    self.reverse = true;
                } else if release && self.reverse {
                    self.reverse = false;
                    self.end_flux(ctx);
                }
            }
            Control::Keylock if press => self.keylock = !self.keylock,
            Control::Tick if press => self.tick = !self.tick,
            Control::KeyShift => {
                if let ControlValue::Absolute(v) = v {
                    self.key_shift = ((v - 0.5) * 24.0).round().clamp(-12.0, 12.0);
                }
            }
            Control::Tempo => {
                if let ControlValue::Absolute(v) = v {
                    self.tempo_fader = v.clamp(0.0, 1.0);
                }
            }
            Control::TempoReset if press => self.tempo_fader = 0.5,
            Control::TempoBendUp => self.bend = if press { 1.0 } else { 0.0 },
            Control::TempoBendDown => self.bend = if press { -1.0 } else { 0.0 },
            Control::Seek => {
                if let ControlValue::Absolute(v) = v {
                    let secs = f64::from(v.clamp(0.0, 1.0)) * self.duration();
                    self.jump(secs, ctx);
                }
            }
            Control::JogTouch => {
                self.jog_touch = press;
                self.scratch_speed = if press { 0.0 } else { self.scratch_speed };
                if press {
                    self.jog_accum = 0.0;
                }
            }
            Control::Jog => {
                if let ControlValue::Delta(d) = v {
                    if self.jog_touch {
                        self.jog_accum += f64::from(d);
                    } else if !self.playing {
                        // Paused: move the track (cueing on a touch strip or
                        // a jog wheel's rim).
                        let to = (self.position() + f64::from(d) * SECS_PER_REV).clamp(0.0, self.duration());
                        self.scrub_to(to);
                    } else if self.sync {
                        // Synced: shift against the master, and keep it.
                        self.jog_offset += f64::from(d) * 0.25;
                        self.nudging = true;
                    } else {
                        self.nudge_secs += f64::from(d) * 0.1;
                    }
                }
            }
            _ => {}
        }
        None
    }

    /// Remix deck transport and slot controls; `false` if `c` is left to
    /// the common deck controls (tempo, keylock, tick…).
    fn remix_control(&mut self, c: Control, v: ControlValue, ctx: &RenderCtx) -> bool {
        let press = matches!(v, ControlValue::Press(true));
        let beat = self.beat().unwrap_or(0.0);
        let Some(r) = self.remix.as_mut() else { return false };
        let cell = match c {
            Control::RemixCell(n) => usize::from(n).checked_sub(1).filter(|&c| c < rille_core::remix::CELLS),
            Control::RemixPad(n) => r.pad_cell(n),
            _ => None,
        };
        match c {
            Control::RemixCell(_) | Control::RemixPad(_) => {
                if let (true, Some(cell)) = (press, cell) {
                    if self.playing {
                        r.trigger(cell, beat, false);
                    } else if r.cells[cell].is_some() {
                        // A stopped deck starts with the sample, on a beat.
                        let b = (beat + 1e-6).floor();
                        if let Some(g) = &self.grid {
                            r.pos = g.secs_at(b);
                        }
                        r.trigger(cell, b, true);
                        self.just_started = true;
                        self.playing = true;
                    }
                }
            }
            Control::Play if press => {
                self.just_started = !self.playing;
                self.playing = !self.playing;
            }
            Control::Cue if press => {
                // Stop and back to bar 1.
                self.playing = false;
                r.stop_all(0);
                r.pos = 0.0;
            }
            Control::Cue | Control::Cup | Control::Play => {}
            Control::TempoBendUp => self.bend = if press { 1.0 } else { 0.0 },
            Control::TempoBendDown => self.bend = if press { -1.0 } else { 0.0 },
            _ => return r.control(c, v, self.index, ctx.declick),
        }
        true
    }

    /// Speed (track seconds per output second) for an unsynced deck, from
    /// the tempo fader, scratching and nudges.
    pub fn free_speed(&mut self, n: usize, sr_out: f64, tempo_range: f64) -> f64 {
        let block_secs = n as f64 / sr_out;
        if self.jog_touch {
            let target = self.jog_accum * SECS_PER_REV / block_secs;
            self.jog_accum = 0.0;
            self.scratch_speed += 0.5 * (target - self.scratch_speed);
            return self.scratch_speed;
        }
        if !self.playing {
            return 0.0;
        }
        let mut s = self.own_rate(tempo_range);
        if self.nudge_secs != 0.0 {
            let step = self.nudge_secs.clamp(-0.3 * block_secs, 0.3 * block_secs);
            self.nudge_secs -= step;
            s += step / block_secs;
        }
        if self.reverse { -s } else { s }
    }

    /// Renders `n` frames at `speed` track seconds per output second into
    /// `self.buf`, wrapping loops. Returns the beats travelled continuously
    /// (loop wraps excluded) and an event if the track ended.
    pub fn render(&mut self, n: usize, speed: f64, key_transpose_ok: bool, ctx: &RenderCtx) -> (f64, Option<Event>) {
        self.speed = speed;
        if self.remix.is_some() {
            return (self.render_remix(n, speed, ctx), None);
        }
        let Some(track) = self.track.clone() else {
            self.buf[..n].fill([0.0; 2]);
            return (0.0, None);
        };
        let sr = f64::from(track.sample_rate);
        let speed_frames = speed * sr / ctx.sr_out;
        let stretch = self.uses_stretch() && key_transpose_ok && speed_frames > 0.05;
        if stretch != self.last_stretch && self.playing {
            // Switching keylock on/off: crossfade to the other path.
            self.last_stretch = stretch;
            let p = self.position();
            self.jump(p, ctx);
        }
        self.last_stretch = stretch;
        let pitch = 2f64.powf(f64::from(self.key_shift) / 12.0) * if self.keylock { 1.0 } else { speed.abs() };
        let transpose = (sr / ctx.sr_out * pitch) as f32;

        self.glide_stems(n, ctx.sr_out);
        // Streaming: wait at the end of what has arrived (with room for the
        // interpolator and the time-stretcher to read ahead). The block that
        // starts the wait still plays, fading out, and the one that ends it
        // fades in: no click either way.
        let was_buffering = std::mem::take(&mut self.buffering);
        if self.streaming.is_some() && self.playing {
            let needed = self.position() + speed.abs() * n as f64 / ctx.sr_out + STREAM_READ_AHEAD_SECS;
            if needed >= track.duration_secs() {
                self.buffering = true;
                if was_buffering {
                    self.buf[..n].fill([0.0; 2]);
                    return (0.0, None);
                }
            }
        }

        let mut beats = 0.0;
        let mut done = 0;
        // Grid beats crossed in this block: (frame offset, downbeat).
        let mut ticks: [Option<(usize, bool)>; 4] = [None; 4];
        while done < n {
            let pos = self.position();
            let mut seg = n - done;
            if self.loop_active
                && speed > 0.0
                && let Some((s, e)) = self.loop_range
                && e > s
            {
                if pos >= e {
                    let len = e - s;
                    let target = s + (pos - e).rem_euclid(len);
                    self.jump(target, ctx);
                    continue;
                }
                let frames_to_end = ((e - pos) * ctx.sr_out / speed).ceil() as usize;
                seg = seg.min(frames_to_end.max(1));
            }
            self.render_voices(&track, done, seg, speed_frames, stretch, transpose, ctx);
            if let Some(g) = &self.grid {
                let (b0, b1) = (g.beat_at(pos), g.beat_at(self.position()));
                beats += b1 - b0;
                // Forward playback only; a jump or scratch doesn't click.
                let k = b0.floor() + 1.0;
                if self.tick && self.playing && b1 > b0 && b1 - b0 < 1.0 && k <= b1 {
                    let at = done + (((k - b0) / (b1 - b0)) * seg as f64) as usize;
                    if let Some(slot) = ticks.iter_mut().find(|t| t.is_none()) {
                        *slot = Some((at.min(done + seg - 1), g.is_downbeat(k as i64)));
                    }
                }
            }
            done += seg;
        }
        for p in self.flux_pos.iter_mut().chain(self.roll.as_mut().map(|r| &mut r.slip)) {
            *p += speed.abs() * n as f64 / ctx.sr_out;
        }
        if self.buffering != was_buffering {
            for (i, f) in self.buf[..n].iter_mut().enumerate() {
                let g = (i as f32 + 0.5) / n as f32;
                let g = if self.buffering { 1.0 - g } else { g };
                f[0] *= g;
                f[1] *= g;
            }
        }

        let mut event = None;
        if self.playing && self.position() >= self.duration() {
            self.playing = false;
            if !self.ended_sent {
                self.ended_sent = true;
                event = Some(Event::TrackEnded { deck: self.index, track_id: self.track_id });
            }
        }
        if !self.playing && !self.jog_touch && self.scratch_speed.abs() < 1e-3 {
            // Paused: output silence but keep the voices where they are.
            self.buf[..n].fill([0.0; 2]);
            self.tick_click = None;
        } else if self.tick || self.tick_click.is_some() {
            self.add_ticks(n, &ticks, ctx.sr_out);
        }
        (beats, event)
    }

    /// Remix deck: moves the virtual playhead and renders the slots. Returns
    /// the beats travelled.
    fn render_remix(&mut self, n: usize, speed: f64, ctx: &RenderCtx) -> f64 {
        let (Some(grid), Some(r)) = (self.grid.clone(), self.remix.as_mut()) else {
            self.buf[..n].fill([0.0; 2]);
            return 0.0;
        };
        let speed = if self.playing { speed.max(0.0) } else { 0.0 };
        let b0 = grid.beat_at(r.pos);
        r.pos += speed * n as f64 / ctx.sr_out;
        let b1 = grid.beat_at(r.pos);
        let bpf = (b1 - b0) / n as f64;
        r.render(&mut self.buf[..n], b0, bpf, self.keylock, ctx);
        let k = b0.floor() + 1.0;
        let mut ticks: [Option<(usize, bool)>; 4] = [None; 4];
        if self.tick && b1 > b0 && k <= b1 {
            let at = (((k - b0) / (b1 - b0)) * n as f64) as usize;
            ticks[0] = Some((at.min(n - 1), grid.is_downbeat(k as i64)));
        }
        if !self.playing {
            self.tick_click = None;
        } else if self.tick || self.tick_click.is_some() {
            self.add_ticks(n, &ticks, ctx.sr_out);
        }
        b1 - b0
    }

    /// Mixes metronome clicks into `self.buf`: a 5 ms tone burst starting
    /// exactly on each grid beat, higher on bar 1.
    fn add_ticks(&mut self, n: usize, ticks: &[Option<(usize, bool)>], sr: f64) {
        let len = (0.005 * sr) as u32;
        for i in 0..n {
            if let Some(&(_, accent)) = ticks.iter().flatten().find(|(at, _)| *at == i) {
                self.tick_click = Some((0, accent));
            }
            let Some((t, accent)) = self.tick_click else { continue };
            let hz = if accent { 2500.0 } else { 1600.0 };
            let x = f64::from(t) / sr;
            let env = (1.0 - f64::from(t) / f64::from(len)).max(0.0);
            let v = (0.5 * env * (std::f64::consts::TAU * hz * x).sin()) as f32;
            self.buf[i][0] += v;
            self.buf[i][1] += v;
            self.tick_click = (t + 1 < len).then_some((t + 1, accent));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_voices(
        &mut self,
        track: &TrackAudio,
        from: usize,
        n: usize,
        speed: f64,
        stretch: bool,
        transpose: f32,
        ctx: &RenderCtx,
    ) {
        let cubic = self.jog_touch;
        let out = &mut self.buf[from..from + n];
        if !self.playing && !self.jog_touch {
            out.fill([0.0; 2]);
            return;
        }
        // With stems away from their full level, the voices read the mix of
        // the stems; a block too fast for the scratch buffer reads the track.
        let gains = self.stem_gain;
        let window = (speed.abs() * n as f64) as usize + 2 * READ_REACH + 2;
        let stems = self.stems.as_deref().filter(|_| gains != [1.0; STEMS] && window <= self.stem_scratch.len());
        let mut mix = stems.map(|stems| StemMix { track, stems, gains, scratch: &mut self.stem_scratch });
        let (active, old) = (self.active, 1 - self.active);
        let old_stretch = stretch && self.voices[old].is_stretching();
        let fading = self.fade_pos < self.fade_len;
        let fade_from = self.fade_from;
        let fade = &mut self.fade_buf[..n];
        let [v0, v1] = &mut self.voices;
        let (voice, old_voice) = if active == 0 { (v0, v1) } else { (v1, v0) };
        match mix.as_mut() {
            Some(m) => voice.render(m, out, speed, stretch, transpose, ctx.sinc, cubic),
            None => voice.render(&mut &*track, out, speed, stretch, transpose, ctx.sinc, cubic),
        }
        if fading {
            match mix.as_mut() {
                Some(m) => old_voice.render(m, fade, speed, old_stretch, transpose, ctx.sinc, cubic),
                None => old_voice.render(&mut &*track, fade, speed, old_stretch, transpose, ctx.sinc, cubic),
            }
            for (i, (o, f)) in out.iter_mut().zip(fade.iter()).enumerate() {
                let g = ((self.fade_pos + i) as f32 / self.fade_len as f32).min(1.0);
                let (gi, go) =
                    ((g * std::f32::consts::FRAC_PI_2).sin(), (g * std::f32::consts::FRAC_PI_2).cos() * fade_from);
                o[0] = o[0] * gi + f[0] * go;
                o[1] = o[1] * gi + f[1] * go;
            }
            self.fade_pos += n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::HOTCUES;

    #[test]
    fn a_jump_during_a_crossfade_does_not_cut_the_fading_voice() {
        // Three flat sections: a cut-off voice shows up as a step.
        const SR: u32 = 48_000;
        let level = |i: usize| match i / SR as usize {
            0 => 0.8,
            1 => 0.0,
            _ => -0.8,
        };
        let frames = (0..3 * SR as usize).map(|i| [level(i); 2]).collect();
        let audio = Arc::new(TrackAudio { sample_rate: SR, frames });
        let track =
            LoadedTrack { id: 1, audio, grid: None, main_cue_secs: 0.0, hotcues: [None; HOTCUES], auto_gain_db: 0.0 };
        let sinc = SincTable::new(32, 256);
        let ctx = RenderCtx { sr_out: f64::from(SR), sinc: &sinc, declick: 144 };
        let mut deck = Deck::new(0, SR, 256);
        deck.load(track, &mut drop);
        deck.set_position(0.5);
        deck.playing = true;
        let mut out = Vec::new();
        let mut render = |deck: &mut Deck, n: usize| {
            deck.render(n, 1.0, true, &ctx);
            out.extend(deck.buf[..n].iter().map(|f| f[0]));
        };
        render(&mut deck, 256);
        // Into the silent section, then (20 frames into that fade) into the
        // negative one, like a jump into phase followed by a loop wrap.
        deck.jump(1.5, &ctx);
        render(&mut deck, 20);
        deck.jump(2.5, &ctx);
        render(&mut deck, 256);
        let step = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(step < 0.05, "largest step between frames {step}");
        assert!((out.last().unwrap() + 0.8).abs() < 1e-3, "lands on the last target");
    }
}
