//! Real bird species, sung from transcriptions: the `bird` generator, and the singers that the
//! `birds` chorus draws on.
//!
//! [`species`] holds each species' song as a score (syllable contours, timbre, grammar),
//! transcribed from CC0 field recordings; no audio is stored. A [`Singer`] performs one bird:
//!
//! * **Voice.** A syrinx-like oscillator: a near-pure tone with the species' harmonics, fast FM
//!   (warble, buzz) and AM (rattle), pitch and level jitter for rough voices, and band-limited
//!   noise. Harsh calls (crow, gull, the owl's "ke-wick") are a full harmonic series shaped by
//!   formants. Where a species sings with both sides of its syrinx, the second contour is a
//!   second, independent oscillator. Woodpecker drumming strikes a [`Modal`] bank instead.
//! * **Grammar.** A song is a sequence of slots; each draws a phrase (a unit of syllables
//!   repeated N times at a period that may speed up, drifting in pitch), and may be skipped.
//!   Songs follow each other after measured pauses, in bouts, with the measured chance of
//!   repeating the same song type.
//! * **Variation.** Every rendition draws its own tempo, pitch, repeat counts, optional slots
//!   and phrase choices, and every syllable its own small pitch and level offset; each
//!   individual has its own voice. No two songs are the same; the species stays recognisable.

pub mod species;

use crate::blocks::{hz_coef, mix_seed, OnePole, Reverb, BLOCK};
use crate::dsp::{balance, Air, Modal};
use crate::filter::{FilterMode, Svf};
use crate::math::{Rng, TAU};
use crate::model::{Generator, InputSpec};
use crate::params::{exp, int, lin, ParamKind, GAIN, UNIT};
use species::{Key, Song, Species, Timbre, SPECIES};

/// Notes that may sound at once (overlapping syllables, echoes of a unit).
const MAX_NOTES: usize = 4;
/// Syllables of one unit waiting for their onset.
const MAX_PENDING: usize = 8;
/// Harmonics of a formant voice.
const MAX_HARM: usize = 28;
/// Edge fade of every note, seconds: no syllable starts or stops with a click.
const EDGE: f32 = 0.0015;
/// Level of a close bird at full amplitude: about -6 dBFS peaks.
const VOICE: f32 = 0.5;

#[inline]
fn smooth01(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Contour value at `u` (0..1): frequency (joined in log-frequency) and amplitude.
#[inline]
///
/// Both are monotone cubic (Fritsch-Carlson) curves through the keypoints, pitch in octaves: the
/// slope is continuous everywhere, so a syllable never has a corner, and the curve never
/// overshoots a keypoint (a peak key is the peak; amplitude never dips below zero).
pub fn contour(keys: &[Key], u: f32) -> (f32, f32) {
    let n = keys.len();
    let first = keys[0];
    if n == 1 || u <= first.0 {
        return (first.1, first.2);
    }
    let last = keys[n - 1];
    if u >= last.0 {
        return (last.1, last.2);
    }
    let mut i = 0;
    while i + 2 < n && u > keys[i + 1].0 {
        i += 1;
    }
    let (a, b) = (keys[i], keys[i + 1]);
    let h = (b.0 - a.0).max(1e-6);
    let x = ((u - a.0) / h).clamp(0.0, 1.0);
    let oct = |k: &Key| k.1.max(1.0).log2();
    // Secant slopes of segment j (per unit u), for the pitch (octaves) and the amplitude.
    let secant = |j: usize, v: &dyn Fn(&Key) -> f32| (v(&keys[j + 1]) - v(&keys[j])) / (keys[j + 1].0 - keys[j].0).max(1e-6);
    let tangent = |k: usize, v: &dyn Fn(&Key) -> f32| -> f32 {
        if k == 0 {
            return secant(0, v);
        }
        if k == n - 1 {
            return secant(n - 2, v);
        }
        let (d0, d1) = (secant(k - 1, v), secant(k, v));
        if d0 * d1 <= 0.0 {
            return 0.0;
        }
        let (h0, h1) = (keys[k].0 - keys[k - 1].0, keys[k + 1].0 - keys[k].0);
        let (w1, w2) = (2.0 * h1 + h0, h1 + 2.0 * h0);
        (w1 + w2) / (w1 / d0 + w2 / d1)
    };
    let amp = |k: &Key| k.2;
    let hermite = |y0: f32, y1: f32, m0: f32, m1: f32| {
        let (x2, x3) = (x * x, x * x * x);
        (2.0 * x3 - 3.0 * x2 + 1.0) * y0 + (x3 - 2.0 * x2 + x) * h * m0 + (-2.0 * x3 + 3.0 * x2) * y1 + (x3 - x2) * h * m1
    };
    let o = hermite(oct(&a), oct(&b), tangent(i, &oct), tangent(i + 1, &oct));
    let g = hermite(a.2, b.2, tangent(i, &amp), tangent(i + 1, &amp)).max(0.0);
    (o.exp2(), g)
}

/// Band-limited random modulation: white noise through a two-pole resonator (Q 1) at the
/// rate, scaled to unit rms, updated once per block. Quasi-periodic like a real bird's
/// vibrato: the rate and depth wander from cycle to cycle.
#[derive(Clone, Copy, Debug, Default)]
struct Wobble {
    y: [f32; 2],
    /// One-pole slow drift (about 3 Hz).
    slow: f32,
}

impl Wobble {
    /// Advance by `dt` seconds; returns (fast, slow) at unit rms each.
    #[inline]
    fn step(&mut self, rate: f32, dt: f32, rng: &mut Rng) -> (f32, f32) {
        let r = (-core::f32::consts::PI * rate * dt).exp();
        let c = (TAU * rate * dt).cos();
        let (a1, a2) = (2.0 * r * c, r * r);
        // Output variance of the resonator for unit-variance input.
        let var = (1.0 + a2) / ((1.0 - a2) * ((1.0 + a2) * (1.0 + a2) - a1 * a1)).max(1e-9);
        let x = rng.next_bipolar() * 1.732;
        let y = a1 * self.y[0] - a2 * self.y[1] + x;
        self.y = [y, self.y[0]];
        let k = 1.0 - (-TAU * 3.0 * dt).exp();
        self.slow += k * (rng.next_bipolar() * 1.732 - self.slow);
        (y / var.sqrt(), self.slow * ((2.0 - k) / k).sqrt())
    }
}

/// Roughly normal, mean 0, sd 1 (sum of three uniforms).
#[inline]
fn gauss(rng: &mut Rng) -> f32 {
    (rng.next_f32() + rng.next_f32() + rng.next_f32() - 1.5) * 2.0
}

/// A log-normal draw around `median` with spread `sigma`.
#[inline]
fn lognormal(rng: &mut Rng, (median, sigma): (f32, f32)) -> f32 {
    median * (sigma * gauss(rng)).exp()
}

/// How a [`Singer`] is driven, block by block.
#[derive(Clone, Copy, Debug)]
pub struct Control {
    /// Whether the bird starts new songs (a song under way always finishes).
    pub awake: bool,
    /// Pauses are this many times their measured length (0.3 = a busy bird).
    pub rest_scale: f32,
    /// 0..1: above about 0.4 the bird gives alarm or excited calls instead of songs.
    pub excitement: f32,
    /// Pitch and tempo multipliers.
    pub pitch: f32,
    pub tempo: f32,
    /// 0..1, 0.5 = renditions vary as measured.
    pub variety: f32,
}

impl Default for Control {
    fn default() -> Self {
        Control { awake: true, rest_scale: 1.0, excitement: 0.0, pitch: 1.0, tempo: 1.0, variety: 0.5 }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Pending {
    /// Song-clock seconds.
    at: f32,
    syl: u8,
    /// Pitch multiplier and level.
    ratio: f32,
    gain: f32,
    /// Duration multiplier.
    stretch: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Note {
    on: bool,
    syl: u8,
    /// Seconds since the onset, and the duration.
    t: f32,
    dur: f32,
    ratio: f32,
    gain: f32,
    /// Samples into the current block before the note starts.
    delay: usize,
    phase: [f32; 2],
    fm: f32,
    am: f32,
    /// This note's FM rate factor (a bird's warble rate wanders from note to note).
    fm_k: f32,
    /// Noise band: two band-passes in a row, so the noise stays near the voice (12 dB/oct).
    band: [Svf; 2],
    /// Jitter (roughness) lowpass state.
    jit: OnePole,
    shim: OnePole,
    /// Micro-modulation of pitch and level, and their values at the end of the last block.
    pitch_wob: Wobble,
    level_wob: Wobble,
    last_wob: (f32, f32),
}

/// Where the singer is in its song.
#[derive(Clone, Copy, Debug, Default)]
struct Run {
    phrase: u8,
    rep: u8,
    reps: u8,
}

/// One bird of one species, singing its songs with natural variation. Mono and unplaced; the
/// `bird` and `birds` generators put it in space.
pub struct Singer {
    sp: &'static Species,
    index: usize,
    sr: f32,
    rng: Rng,
    noise: Rng,
    /// This individual's pitch (octaves) and pace.
    own_pitch: f32,
    own_tempo: f32,
    singing: bool,
    alarm: bool,
    /// Nominal seconds of rest left (scaled by `rest_scale` as it passes).
    rest: f32,
    /// Song clock and the start of the next unit, in seconds.
    clock: f32,
    cursor: f32,
    song: usize,
    last_song: usize,
    slot: usize,
    lap: u8,
    laps: u8,
    run: Option<Run>,
    /// This rendition's pitch (ratio) and tempo (period multiplier).
    ratio: f32,
    pace: f32,
    bout_left: u8,
    pending: [Pending; MAX_PENDING],
    n_pending: usize,
    notes: [Note; MAX_NOTES],
    modal: Modal,
    /// The beak's contact pulse: samples until it starts, samples left of it, its force.
    knock: (usize, usize, f32),
    modal_awake: bool,
    weights: [f32; MAX_HARM],
}

impl Singer {
    /// A bird of species `index` (into [`SPECIES`]); `seed` makes the individual.
    pub fn new(index: usize, seed: u32, sr: f32) -> Self {
        let index = index.min(SPECIES.len() - 1);
        let mut rng = Rng::new(mix_seed(seed));
        let sp = SPECIES[index];
        let own_pitch = sp.var.individual * gauss(&mut rng) * 0.35;
        let own_tempo = 1.0 + 0.06 * gauss(&mut rng);
        // Start part-way into a rest, so birds created together do not sing together.
        let rest = rng.range(0.05, 1.0) * sp.pause.0.min(6.0);
        Singer {
            sp,
            index,
            sr,
            noise: Rng::new(mix_seed(seed ^ 0x51F1_5EED)),
            rng,
            own_pitch,
            own_tempo,
            singing: false,
            alarm: false,
            rest,
            clock: 0.0,
            cursor: 0.0,
            song: 0,
            last_song: usize::MAX,
            slot: 0,
            lap: 0,
            laps: 1,
            run: None,
            ratio: 1.0,
            pace: 1.0,
            bout_left: 0,
            pending: [Pending::default(); MAX_PENDING],
            n_pending: 0,
            notes: [Note::default(); MAX_NOTES],
            modal: Modal::new(&KNOCK_RATIOS, &KNOCK_T60, &KNOCK_LEVEL, seed ^ 0xC0FF),
            knock: (0, 0, 0.0),
            modal_awake: false,
            weights: [0.0; MAX_HARM],
        }
    }

    /// Start the next song within `secs` (a bird that has just appeared sings soon).
    pub fn hurry(&mut self, secs: f32) {
        if !self.singing {
            self.rest = self.rest.min(self.rng.range(0.0, secs));
        }
    }

    pub fn species(&self) -> usize {
        self.index
    }

    pub fn species_data(&self) -> &'static Species {
        self.sp
    }

    /// Whether a song (or its last note) is under way.
    pub fn is_busy(&self) -> bool {
        self.singing || self.notes.iter().any(|n| n.on) || self.modal_awake
    }

    /// Become a bird of another species (keeps the individual's voice). Best done while idle.
    pub fn set_species(&mut self, index: usize) {
        let index = index.min(SPECIES.len() - 1);
        if index != self.index {
            self.index = index;
            self.sp = SPECIES[index];
            self.singing = false;
            self.run = None;
            self.n_pending = 0;
            self.notes.iter_mut().for_each(|n| n.on = false);
            self.last_song = usize::MAX;
            self.bout_left = 0;
            self.rest = self.rest.min(self.rng.range(0.2, 2.0));
        }
    }

    fn pick_song(&mut self, songs: &[Song]) -> usize {
        if songs.len() == 1 {
            return 0;
        }
        if self.last_song < songs.len() && self.rng.chance(self.sp.repeat) {
            return self.last_song;
        }
        let total: f32 = songs.iter().enumerate().filter(|(i, _)| *i != self.last_song).map(|(_, s)| s.weight).sum();
        let mut x = self.rng.next_f32() * total;
        for (i, s) in songs.iter().enumerate() {
            if i == self.last_song {
                continue;
            }
            x -= s.weight;
            if x <= 0.0 {
                return i;
            }
        }
        songs.len() - 1
    }

    fn draw(&mut self, (lo, hi): (u8, u8)) -> u8 {
        let span = hi.saturating_sub(lo) as f32 + 1.0;
        lo + ((self.rng.next_f32() * span) as u8).min(hi.saturating_sub(lo))
    }

    fn songs(&self) -> &'static [Song] {
        if self.alarm {
            self.sp.alarm
        } else {
            self.sp.songs
        }
    }

    fn start_song(&mut self, c: &Control) {
        let sp = self.sp;
        let alarm_p = smooth01((c.excitement - 0.3) / 0.4);
        self.alarm = !sp.alarm.is_empty() && self.rng.chance(alarm_p);
        let songs = self.songs();
        self.song = self.pick_song(songs);
        if !self.alarm {
            self.last_song = self.song;
        }
        self.laps = self.draw(songs[self.song].laps).max(1);
        self.lap = 0;
        self.slot = 0;
        self.run = None;
        let v = c.variety * 2.0;
        self.ratio = c.pitch * (self.own_pitch + v * sp.var.pitch * gauss(&mut self.rng)).exp2();
        self.pace = (self.own_tempo * (1.0 + v * sp.var.tempo * gauss(&mut self.rng))).clamp(0.6, 1.6) / c.tempo.max(0.1);
        self.clock = 0.0;
        self.cursor = 0.0;
        self.singing = true;
    }

    /// Choose the next phrase from the song's slots; false when the song is over.
    fn next_phrase(&mut self) -> bool {
        let song = self.songs()[self.song];
        loop {
            if self.slot >= song.slots.len() {
                self.lap += 1;
                self.slot = 0;
                if self.lap >= self.laps {
                    return false;
                }
            }
            let s = song.slots[self.slot];
            self.slot += 1;
            if s.phrases.is_empty() || !self.rng.chance(s.p) {
                continue;
            }
            let k = ((self.rng.next_f32() * s.phrases.len() as f32) as usize).min(s.phrases.len() - 1);
            let phrase = s.phrases[k];
            let reps = self.draw(self.sp.phrases[phrase as usize].reps).max(1);
            self.run = Some(Run { phrase, rep: 0, reps });
            return true;
        }
    }

    /// Queue the syllables of the next unit; false when the song is over.
    fn plan_unit(&mut self, c: &Control) -> bool {
        if self.run.is_none() && !self.next_phrase() {
            return false;
        }
        let mut run = self.run.unwrap();
        let ph = self.sp.phrases[run.phrase as usize];
        let x = if run.reps > 1 { run.rep as f32 / (run.reps - 1) as f32 } else { 0.0 };
        let shift = (ph.drift * x).exp2();
        let level = 1.0 + (ph.swell - 1.0) * x;
        let v = c.variety * 2.0;
        // Syllables stretch less than the gaps between them when a bird sings faster.
        let stretch = self.pace.sqrt();
        let mut end = 0.0f32;
        for &(syl, off) in ph.unit {
            if self.n_pending >= MAX_PENDING {
                break;
            }
            let d = self.sp.syllables[syl as usize].dur * stretch;
            let at = self.cursor + off * self.pace;
            end = end.max(at + d);
            let jitter = (v * self.sp.var.note * gauss(&mut self.rng)).exp2();
            let gain = level * (1.0 - 0.12 * v * self.rng.next_f32());
            self.pending[self.n_pending] = Pending { at, syl, ratio: self.ratio * shift * jitter, gain, stretch };
            self.n_pending += 1;
        }
        run.rep += 1;
        if run.rep >= run.reps {
            self.cursor = end + ph.gap * self.pace;
            self.run = None;
        } else {
            let period = ph.period.0 + (ph.period.1 - ph.period.0) * x;
            self.cursor += period * self.pace;
            self.run = Some(run);
        }
        true
    }

    fn end_song(&mut self) {
        self.singing = false;
        let sp = self.sp;
        if self.alarm {
            self.rest = lognormal(&mut self.rng, sp.alarm_pause);
            return;
        }
        self.rest = lognormal(&mut self.rng, sp.pause);
        if sp.bout.1 > 0 {
            if self.bout_left <= 1 {
                self.bout_left = self.draw(sp.bout).max(1);
                self.rest = lognormal(&mut self.rng, sp.bout_pause);
            } else {
                self.bout_left -= 1;
            }
        }
    }

    fn start_note(&mut self, p: Pending, delay: usize) {
        let syl = self.sp.syllables[p.syl as usize];
        let timbre = self.sp.timbres[syl.timbre as usize];
        if timbre.knock {
            self.knock = (delay, KNOCK_SAMPLES, p.gain * syl.keys[0].2);
            self.modal.set(syl.keys[0].1 * p.ratio, 0.5, 1.0, self.sr);
            return;
        }
        // Take a free slot, or the oldest note.
        let k = self.notes.iter().position(|n| !n.on).unwrap_or_else(|| {
            let mut best = 0;
            for (i, n) in self.notes.iter().enumerate() {
                if n.t > self.notes[best].t {
                    best = i;
                }
            }
            best
        });
        let n = &mut self.notes[k];
        *n = Note { on: true, syl: p.syl, t: 0.0, dur: syl.dur * p.stretch, ratio: p.ratio, gain: p.gain, delay, ..*n };
        n.phase = [0.0, 0.0];
        n.fm = 0.0;
        n.am = 0.0;
        n.fm_k = 1.0 + 0.1 * self.rng.next_bipolar();
        n.pitch_wob = Wobble::default();
        n.level_wob = Wobble::default();
        n.last_wob = (1.0, 1.0);
    }

    /// Render one block (`out.len() <= BLOCK`), adding the bird's dry voice to `out`.
    pub fn render(&mut self, out: &mut [f32], c: &Control) {
        let n = out.len();
        let dt = n as f32 / self.sr;
        if !self.singing && !self.notes.iter().any(|n| n.on) {
            self.rest -= dt / c.rest_scale.max(0.05);
            if self.rest <= 0.0 {
                if c.awake {
                    self.start_song(c);
                } else {
                    self.rest = self.rng.range(0.2, 1.0);
                }
            }
        }
        if self.singing {
            let t1 = self.clock + dt;
            loop {
                // Start every queued syllable whose onset falls in this block.
                let mut i = 0;
                while i < self.n_pending {
                    let p = self.pending[i];
                    if p.at < t1 {
                        let delay = (((p.at - self.clock).max(0.0)) * self.sr) as usize;
                        self.start_note(p, delay.min(n - 1));
                        self.n_pending -= 1;
                        self.pending[i] = self.pending[self.n_pending];
                    } else {
                        i += 1;
                    }
                }
                if self.n_pending > 0 || self.cursor >= t1 {
                    break;
                }
                if !self.plan_unit(c) {
                    self.end_song();
                    break;
                }
            }
            self.clock = t1;
        }
        for k in 0..MAX_NOTES {
            if self.notes[k].on {
                self.render_note(k, out);
            }
        }
        self.render_knock(out);
    }

    fn render_knock(&mut self, out: &mut [f32]) {
        let (mut delay, mut left, gain) = self.knock;
        if left == 0 && !self.modal_awake {
            return;
        }
        // The beak's contact: a half-sine force pulse of 1 ms (at 48 kHz), a little rough. It may
        // run on into the next block.
        for o in out.iter_mut() {
            let mut x = 0.0;
            if delay > 0 {
                delay -= 1;
            } else if left > 0 {
                let w = (core::f32::consts::PI * left as f32 / KNOCK_SAMPLES as f32).sin();
                x = gain * w * (0.8 + 0.2 * self.noise.next_bipolar());
                left -= 1;
            }
            *o += self.modal.tick(x, 0.3, 0.25, self.sr) * 0.045;
        }
        self.knock = (delay, left, gain);
        self.modal_awake = left > 0 || self.modal.end_block() > 1e-5;
    }

    fn render_note(&mut self, k: usize, out: &mut [f32]) {
        let sr = self.sr;
        let sp = self.sp;
        let note = self.notes[k];
        let syl = sp.syllables[note.syl as usize];
        let tb: Timbre = sp.timbres[syl.timbre as usize];
        let n = out.len();
        let start = note.delay.min(n);
        let len = n - start;
        let t0 = note.t;
        let t1 = t0 + len as f32 / sr;
        let (u0, u1) = (t0 / note.dur, (t1 / note.dur).min(1.0));
        let edge = |t: f32| smooth01(t / EDGE) * smooth01((note.dur - t) / EDGE);
        let gain = note.gain * sp.level * VOICE;
        // Micro-modulation over this block: pitch and level factors at its start and end.
        let m = sp.micro;
        let depth = m.cents * if tb.wobble > 0.0 { tb.wobble } else { 1.0 };
        let (w0, l0) = self.notes[k].last_wob;
        let (w1, l1) = if m.cents > 0.0 || m.flutter > 0.0 {
            let dt = len as f32 / sr;
            let nz = &mut self.noise;
            let note = &mut self.notes[k];
            let (pf, ps) = note.pitch_wob.step(m.rate, dt, nz);
            let (lf, _) = note.level_wob.step(m.rate * 0.7, dt, nz);
            let cents = depth * (0.9 * pf + 0.45 * ps);
            // Level flutter is held within +-2 sd, so a rare draw never peaks a note by +6 dB.
            // (Its mean is taken off, so flutter adds life without raising the level.)
            ((cents / 1200.0).exp2(), crate::math::db_to_gain(m.flutter * (lf.clamp(-2.0, 2.0) - 0.7)))
        } else {
            (1.0, 1.0)
        };
        self.notes[k].last_wob = (w1, l1);
        let sides = if syl.keys2.is_empty() { 1 } else { 2 };
        let formant = !tb.formants.is_empty();
        // Both sides of the syrinx share the note's FM and AM.
        let (fm_start, am_start) = (self.notes[k].fm, self.notes[k].am);
        for side in 0..sides {
            let keys = if side == 0 { syl.keys } else { syl.keys2 };
            let (fa, aa) = contour(keys, u0);
            let (fb, ab) = contour(keys, u1);
            let (fa, fb) = (fa * note.ratio * w0, fb * note.ratio * w1);
            let (ga, gb) = (aa * edge(t0) * gain * l0, ab * edge(t1) * gain * l1);
            if formant {
                self.set_weights(&tb, 0.5 * (fa + fb));
            }
            let nh = if formant { ((0.45 * sr / fb.max(fa).max(20.0)) as usize).min(MAX_HARM) } else { 4 };
            // Harmonics fade out between 9 and 12 kHz (and always below Nyquist): a bird's upper
            // harmonics are weak up there, and the air takes the rest. The fade is smooth in
            // frequency, so a sweeping note never switches one on with a step.
            let top = fa.max(fb).max(20.0);
            let fade = |k: f32| {
                let hz = k * top;
                if hz > 0.45 * sr {
                    0.0
                } else {
                    smooth01((12000.0 - hz) / 3000.0)
                }
            };
            let h = [1.0, tb.harm[0] * fade(2.0), tb.harm[1] * fade(3.0), tb.harm[2] * fade(4.0)];
            let note = &mut self.notes[k];
            let mut noise_gain = 0.0;
            if tb.noise > 0.0 {
                let centre = (if formant { tb.formants[0].0 } else { 0.5 * (fa + fb) }).min(0.4 * sr);
                // Per stage k = 1 / Q; the pair is about 0.64 times as wide as one stage.
                let k = if tb.band > 0.0 { (tb.band / 0.64).min(1.9) } else if formant { 1.21 } else { 0.61 };
                let res = (2.0 - k) / 1.98;
                note.band.iter_mut().for_each(|b| b.set(FilterMode::BandPass, centre, res, sr));
                // Scale the band to the level of a sine (rms 0.707): each SVF peaks at 1 / k, and
                // white noise (rms 0.577) keeps the share (pi / 4) bw / (sr / 2) of its power
                // through the pair.
                let bw = centre * k;
                noise_gain = k * k * 0.707 / (0.577 * (core::f32::consts::PI * bw / (2.0 * sr)).sqrt());
            }
            let jit_c = hz_coef(120.0, sr);
            let inv = 1.0 / len.max(1) as f32;
            let mut ph = note.phase[side];
            note.fm = fm_start;
            note.am = am_start;
            let fm_rate = tb.fm.0 * note.fm_k;
            for i in 0..len {
                let x = i as f32 * inv;
                let mut f = fa + (fb - fa) * x;
                let mut a = ga + (gb - ga) * x;
                if tb.fm.1 > 0.0 {
                    note.fm = (note.fm + fm_rate / sr).fract();
                    f *= (tb.fm.1 * (TAU * note.fm).sin()).exp2();
                }
                if tb.am.1 > 0.0 {
                    note.am = (note.am + tb.am.0 / sr).fract();
                    a *= 1.0 - tb.am.1 * (0.5 + 0.5 * (TAU * note.am).cos());
                }
                if tb.rough > 0.0 {
                    let j = note.jit.lp(self.noise.next_bipolar(), jit_c) * 4.0;
                    let s = note.shim.lp(self.noise.next_bipolar(), jit_c) * 4.0;
                    f *= 1.0 + 0.06 * tb.rough * j;
                    a *= (1.0 + 0.8 * tb.rough * s).max(0.0);
                }
                ph += (f / sr).min(0.45);
                if ph >= 1.0 {
                    ph -= 1.0;
                }
                let th = TAU * ph;
                let (s1, c1) = th.sin_cos();
                let mut y;
                if formant {
                    // Chebyshev recurrence: sin(k th) from sin(th) and cos(th).
                    let c2 = 2.0 * c1;
                    let (mut prev, mut cur) = (0.0f32, s1);
                    y = 0.0;
                    for w in &self.weights[..nh] {
                        y += w * cur;
                        let next = c2 * cur - prev;
                        prev = cur;
                        cur = next;
                    }
                } else {
                    let s2 = 2.0 * c1 * s1;
                    let s3 = 2.0 * c1 * s2 - s1;
                    let s4 = 2.0 * c1 * s3 - s2;
                    y = h[0] * s1 + h[1] * s2 + h[2] * s3 + h[3] * s4;
                }
                if tb.noise > 0.0 {
                    let z = note.band[0].tick(self.noise.next_bipolar());
                    let z = note.band[1].tick(z) * noise_gain;
                    y = y * (1.0 - tb.noise) + z * tb.noise;
                }
                out[start + i] += y * a;
            }
            note.phase[side] = ph;
        }
        let note = &mut self.notes[k];
        note.delay = 0;
        note.t = t1;
        if t1 >= note.dur {
            note.on = false;
            note.band.iter_mut().for_each(Svf::reset);
        }
    }

    /// Harmonic weights of a formant voice at fundamental `f0`, normalised to unit power.
    fn set_weights(&mut self, tb: &Timbre, f0: f32) {
        let mut power = 0.0;
        for (k, w) in self.weights.iter_mut().enumerate() {
            let hz = f0 * (k + 1) as f32;
            let mut g = 0.0;
            for &(fc, bw, level) in tb.formants {
                let d = (hz - fc) / (0.5 * bw);
                g += level / (1.0 + d * d);
            }
            if hz > 0.45 * self.sr {
                g = 0.0;
            }
            *w = g;
            power += g * g;
        }
        let norm = 1.0 / power.sqrt().max(1e-6);
        self.weights.iter_mut().for_each(|w| *w *= norm);
    }
}

/// Modes of a branch knocked by a beak (ratio to the strongest, ring time, level), measured on
/// drumming recordings (Freesound 428146: peaks at 1.03, 0.69, 1.32, 0.28, 2.07 and 2.86 kHz,
/// at 0, -12, -11, -20, -23 and -24 dB). The species table tunes the strongest mode.
/// Length of the contact pulse in samples.
const KNOCK_SAMPLES: usize = 48;
const KNOCK_RATIOS: [f32; 6] = [1.0, 0.67, 1.28, 0.27, 2.0, 2.78];
const KNOCK_T60: [f32; 6] = [0.06, 0.07, 0.05, 0.09, 0.035, 0.03];
const KNOCK_LEVEL: [f32; 6] = [1.0, 0.3, 0.3, 0.12, 0.08, 0.07];

// ---------------------------------------------------------------------------------------------
// The `bird` generator
// ---------------------------------------------------------------------------------------------

model_params! {
    /// One bird of a real species, singing transcribed songs with natural variation.
    BirdParams / BirdParamId {
        species: "bird/species" = 0.0, ParamKind::Enum(species::NAMES);
        individual: "bird/individual" = 0.0, int(0, 999);
        pitch: "song/pitch" = 1.0, exp(0.7, 1.4);
        tempo: "song/tempo" = 1.0, exp(0.6, 1.6);
        variety: "song/variety" = 0.5, UNIT;
        pan: "space/pan" = 0.0, lin(-1.0, 1.0);
        reverb: "space/reverb" = 0.3, UNIT;
        rt60: "space/rt60_s" = 1.2, lin(0.2, 4.0);
        width: "space/width" = 0.6, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

pub struct Bird {
    sr: f32,
    singer: Singer,
    /// (species, individual) the singer was made for.
    made: (usize, u32),
    air: Air,
    reverb: Reverb,
}

impl Bird {
    fn render(&mut self, x: &[f32], p: &BirdParams, width: f32, left: &mut [f32], right: &mut [f32]) {
        let want = (p.species as usize, p.individual as u32);
        if want != self.made && !self.singer.is_busy() {
            self.made = want;
            self.singer = Singer::new(want.0, 0xB1D0_0000 ^ want.1.wrapping_mul(0x9E37), self.sr);
        }
        let (activity, distance, excitement) = (x[0], x[1], x[2]);
        let c = Control {
            awake: activity > 0.02,
            // Pauses as measured at activity 0.6, 0.4x at 1, 5x at 0.1. Excited birds call more
            // often, too.
            rest_scale: (1.0 - 0.6 * excitement) / (0.15 + 2.36 * activity * activity),
            excitement,
            pitch: p.pitch,
            tempo: p.tempo,
            variety: p.variety,
        };
        let n = left.len();
        let mut dry = [0.0f32; BLOCK];
        self.singer.render(&mut dry[..n], &c);
        let d = distance.clamp(0.0, 1.0);
        // A single bird is placed by the game, whose 3D player attenuates it with distance; this
        // is what the air and the woods add: the top falls away (16 -> 5 kHz), the reverb grows
        // and the direct sound drops 5 dB.
        self.air.set(d, 16000.0, 5000.0, 1.0 / (1.0 + 0.8 * d), self.sr);
        let send = (0.15 + 0.85 * d) * p.reverb * 1.2;
        let (gl, gr) = balance(p.pan * if width > 0.0 { 1.0 } else { 0.0 });
        for i in 0..n {
            let y = self.air.tick(dry[i]);
            let (m, s) = self.reverb.tick_stereo(y * send, p.rt60, 0.45);
            left[i] = (y * gl + m + s * width) * p.gain;
            right[i] = (y * gr + m - s * width) * p.gain;
        }
    }
}

impl Generator for Bird {
    type P = BirdParams;
    const NAME: &'static str = "bird";
    const CATEGORY: &'static str = "nature";
    const DOC: &'static str = "One bird of a real species (blackbird, robin, chaffinch, cuckoo, crow, tawny owl…), singing songs transcribed from field recordings, never the same twice; alarm calls when excited.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "activity", default: 0.6, doc: "How often it sings: 0 = silent, 0.6 = as recorded, 1 = eager" },
        InputSpec { name: "distance", default: 0.2, doc: "0 = close by, 1 = far away (quieter, duller, more reverb)" },
        InputSpec { name: "excitement", default: 0.0, doc: "0 = song; towards 1, alarm or excited calls instead" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.2;

    fn presets() -> Vec<(&'static str, BirdParams)> {
        SPECIES.iter().enumerate().map(|(i, s)| (s.name, BirdParams { species: i as f32, ..Default::default() })).collect()
    }

    fn snap(&mut self, x: &[f32], p: &BirdParams) {
        // Spawned mid-scene: be the bird it should be, and sing soon if active.
        let want = (p.species as usize, p.individual as u32);
        if want != self.made {
            self.made = want;
            self.singer = Singer::new(want.0, 0xB1D0_0000 ^ want.1.wrapping_mul(0x9E37), self.sr);
        }
        if x[0] > 0.3 {
            self.singer.hurry(0.8);
        }
    }

    fn new(sr: f32) -> Self {
        Bird { sr, singer: Singer::new(0, 0xB1D0_0000, sr), made: (0, 0), air: Air::default(), reverb: Reverb::new(sr) }
    }

    fn block(&mut self, x: &[f32], p: &BirdParams, out: &mut [f32]) {
        let mut right = [0.0f32; BLOCK];
        let n = out.len();
        self.render(x, p, 0.0, out, &mut right[..n]);
    }

    fn block_stereo(&mut self, x: &[f32], p: &BirdParams, left: &mut [f32], right: &mut [f32]) {
        self.render(x, p, p.width, left, right);
    }
}
