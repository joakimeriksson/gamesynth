//! A single synthesizer voice: 3 oscillators -> filter -> amplifier, with two envelopes,
//! an LFO and the sfxr-style pitch modulators (slide, arp jump, vibrato).

use crate::env::Adsr;
use crate::filter::Svf;
use crate::lfo::Lfo;
use crate::math::{midi_to_hz, onepole_coef, TAU};
use crate::osc::Oscillator;
use crate::patch::Patch;

/// Number of samples between control-rate updates (envelopes are still per-sample).
pub const CONTROL_BLOCK: usize = 32;

#[derive(Clone, Copy, Debug)]
pub struct Voice {
    active: bool,
    /// Rounded note used to match note-off events.
    key: i32,
    note: f32,
    cur_note: f32,
    velocity: f32,
    /// Seconds since note-on.
    t: f32,
    /// Samples until automatic release, negative when disabled.
    auto_release: i64,
    /// Monotonic counter for voice stealing (lower = older).
    age: u64,
    osc: [Oscillator; 3],
    dt: [f32; 3],
    filter: Svf,
    amp_env: Adsr,
    filter_env: Adsr,
    lfo: Lfo,
}

impl Voice {
    pub fn new(seed: u32) -> Self {
        Voice {
            active: false,
            key: -1,
            note: 60.0,
            cur_note: 60.0,
            velocity: 1.0,
            t: 0.0,
            auto_release: -1,
            age: 0,
            osc: [
                Oscillator::new(seed),
                Oscillator::new(seed.wrapping_mul(7).wrapping_add(11)),
                Oscillator::new(seed.wrapping_mul(13).wrapping_add(17)),
            ],
            dt: [0.0; 3],
            filter: Svf::default(),
            amp_env: Adsr::default(),
            filter_env: Adsr::default(),
            lfo: Lfo::default(),
        }
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[inline]
    pub fn is_released(&self) -> bool {
        self.amp_env.is_released()
    }

    #[inline]
    pub fn key(&self) -> i32 {
        self.key
    }

    #[inline]
    pub fn age(&self) -> u64 {
        self.age
    }

    #[inline]
    pub fn level(&self) -> f32 {
        self.amp_env.level()
    }

    /// Start (or legato-retrigger) the voice.
    ///
    /// `duration` > 0 schedules an automatic release; `legato` keeps oscillator phase and
    /// filter state and glides from the current pitch instead of restarting.
    #[allow(clippy::too_many_arguments)]
    pub fn note_on(
        &mut self,
        p: &Patch,
        sample_rate: f32,
        note: f32,
        velocity: f32,
        duration: f32,
        age: u64,
        legato: bool,
    ) {
        let velocity = velocity.clamp(0.0, 1.0);
        if !legato || !self.active {
            self.cur_note = note;
            self.t = 0.0;
            self.filter.reset();
            self.lfo.reset();
            for (i, o) in self.osc.iter_mut().enumerate() {
                o.reset(0.0);
                self.dt[i] = 0.0;
            }
            self.amp_env.trigger(&p.amp_env, sample_rate);
            self.filter_env.trigger(&p.filter_env, sample_rate);
        } else if self.amp_env.is_released() {
            self.amp_env.trigger(&p.amp_env, sample_rate);
            self.filter_env.trigger(&p.filter_env, sample_rate);
        }
        self.note = note;
        self.key = note.round() as i32;
        self.velocity = velocity;
        self.age = age;
        self.active = true;
        self.auto_release = if duration > 0.0 { (duration * sample_rate) as i64 } else { -1 };
    }

    pub fn release(&mut self) {
        self.amp_env.release();
        self.filter_env.release();
        self.auto_release = -1;
    }

    pub fn kill(&mut self) {
        self.active = false;
        self.amp_env.kill();
        self.filter_env.kill();
        self.filter.reset();
    }

    /// Render and *add* this voice into `out`. `bend` is the global pitch bend in semitones.
    pub fn render(&mut self, p: &Patch, sample_rate: f32, bend: f32, out: &mut [f32]) {
        if !self.active {
            return;
        }
        let mut offset = 0;
        while offset < out.len() {
            let n = (out.len() - offset).min(CONTROL_BLOCK);
            self.render_block(p, sample_rate, bend, &mut out[offset..offset + n]);
            offset += n;
            if !self.active {
                break;
            }
        }
    }

    fn render_block(&mut self, p: &Patch, sample_rate: f32, bend: f32, out: &mut [f32]) {
        let n = out.len();
        let dt_block = n as f32 / sample_rate;
        let t = self.t;

        // --- control rate --------------------------------------------------------------
        if p.pitch.glide > 0.0 {
            let coef = onepole_coef(p.pitch.glide, sample_rate / n as f32);
            self.cur_note += (self.note - self.cur_note) * coef;
        } else {
            self.cur_note = self.note;
        }

        let lfo_fade = if p.lfo.fade_in > 0.0 { (t / p.lfo.fade_in).min(1.0) } else { 1.0 };
        let lfo = if p.lfo.is_active() {
            self.lfo.advance(p.lfo.wave, p.lfo.rate_hz, dt_block) * lfo_fade
        } else {
            0.0
        };

        let slide = p.pitch.slide * t + 0.5 * p.pitch.slide_accel * t * t;
        let arp = if p.pitch.arp_time > 0.0 && t >= p.pitch.arp_time { p.pitch.arp_semitones } else { 0.0 };
        let vibrato = if p.pitch.vibrato_depth > 0.0 && p.pitch.vibrato_rate > 0.0 {
            p.pitch.vibrato_depth * (TAU * p.pitch.vibrato_rate * t).sin()
        } else {
            0.0
        };
        let pitch_mod = slide + arp + vibrato + lfo * p.lfo.pitch + bend;

        let cutoff_oct = self.filter_env.level() * p.filter.env_amount
            + p.filter.key_track * (self.cur_note - 60.0) / 12.0
            + p.filter.sweep * t
            + lfo * p.lfo.cutoff
            + p.filter.velocity * (self.velocity - 1.0);
        self.filter.set(p.filter.mode, p.filter.cutoff_hz * cutoff_oct.exp2(), p.filter.resonance, sample_rate);

        let mut dt_target = [0.0f32; 3];
        let mut pw = [0.5f32; 3];
        for i in 0..3 {
            let o = &p.osc[i];
            if o.level <= 0.0 || o.wave.is_noise() {
                continue;
            }
            let hz = midi_to_hz(self.cur_note + pitch_mod + o.semitones + o.detune_cents * 0.01);
            dt_target[i] = hz / sample_rate;
            if self.dt[i] == 0.0 {
                self.dt[i] = dt_target[i];
            }
            pw[i] = (o.pulse_width + o.pulse_width_sweep * t + lfo * p.lfo.pulse_width).clamp(0.02, 0.98);
        }
        let dt_step = [
            (dt_target[0] - self.dt[0]) / n as f32,
            (dt_target[1] - self.dt[1]) / n as f32,
            (dt_target[2] - self.dt[2]) / n as f32,
        ];

        let tremolo = 1.0 - p.lfo.amp * (0.5 + 0.5 * lfo);
        let vel_gain = (1.0 - p.velocity_sens) + p.velocity_sens * self.velocity;
        let gain = vel_gain * tremolo;

        // --- audio rate ----------------------------------------------------------------
        for s in out.iter_mut() {
            let mut mix = 0.0;
            for i in 0..3 {
                let o = &p.osc[i];
                if o.level <= 0.0 {
                    continue;
                }
                self.dt[i] += dt_step[i];
                mix += self.osc[i].next(o.wave, self.dt[i], pw[i]) * o.level;
            }
            self.filter_env.tick();
            let amp = self.amp_env.tick();
            *s += self.filter.tick(mix) * amp * gain;
        }

        // --- bookkeeping ---------------------------------------------------------------
        self.t += dt_block;
        if self.auto_release >= 0 {
            self.auto_release -= n as i64;
            if self.auto_release <= 0 {
                self.release();
            }
        }
        if self.amp_env.is_idle() {
            self.active = false;
        }
    }
}

impl Default for Voice {
    fn default() -> Self {
        Voice::new(0x5EED_0001)
    }
}
