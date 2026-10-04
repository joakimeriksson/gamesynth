//! Sounds of sport that run all match long. (The one-shot ones are recipes in [`super::fx`];
//! the crowd is [`super::ambient::Crowd`].)

use crate::blocks::{Dust, SlowNoise};
use crate::filter::{FilterMode, Svf};
use crate::math::Rng;
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::params::{exp, lin, GAIN, UNIT};

model_params! {
    /// One skater's blades on ice.
    ///
    /// Measured on a recording of skating (`tools/reference/hockey`): each stride is a burst
    /// of broad noise about a second long, level between 500 Hz and 4 kHz, whose loudness
    /// flutters by a third as the blade shaves. Between strides there is almost nothing.
    /// `blade/top_hz` sets how much of that top the game keeps: the default is duller than
    /// the recording so that four skaters for a whole match do not add up to hiss; the
    /// "Close up" preset is the recording.
    SkateParams / SkateParamId {
        stride_hz: "stride/hz" = 1.3, lin(0.5, 3.0);
        cut_level: "stride/cut" = 0.8, UNIT;
        blade_hz: "blade/hz" = 1100.0, exp(400.0, 3000.0);
        top_hz: "blade/top_hz" = 2800.0, exp(1000.0, 8000.0);
        grain: "blade/grain" = 0.5, UNIT;
        glide_level: "glide/level" = 0.3, UNIT;
        sing_hz: "glide/sing_hz" = 1900.0, exp(800.0, 4000.0);
        spray_level: "edge/spray" = 0.8, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

pub struct Skate {
    sr: f32,
    noise: Noise,
    rng: Rng,
    /// Position in the stride, 0..1; the blade is on the ice for the first part of it.
    phase: f32,
    /// This stride's own pitch and strength: no two pushes are the same.
    stride_pitch: f32,
    stride_power: f32,
    stride_pace: f32,
    blade: Svf,
    top: Svf,
    flutter: SlowNoise,
    rough: SlowNoise,
    sing: Svf,
    hush: Svf,
    spray: Svf,
    spray_top: Svf,
    chatter: SlowNoise,
    chips: Dust,
    chip_env: f32,
    chip_soft: f32,
    chip_bp: Svf,
}

impl Generator for Skate {
    type P = SkateParams;
    const NAME: &'static str = "skate";
    const CATEGORY: &'static str = "sport";
    const DOC: &'static str = "A skater's blades on ice: the cut of each stride, a faint singing glide, a thick spray on the edges. One per skater.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "speed", default: 0.0, doc: "0 standing, 1 flat out" },
        InputSpec { name: "push", default: 1.0, doc: "1 while striding, 0 gliding" },
        InputSpec { name: "edge", default: 0.0, doc: "Carving hard or stopping: the spray of shaved ice" },
        InputSpec { name: "surface", default: 0.0, doc: "0 fresh ice, 1 cut-up late-period ice: rougher and grainier" },
    ];
    const INPUT_SMOOTH_SECS: f32 = 0.05;

    fn presets() -> Vec<(&'static str, SkateParams)> {
        vec![
            // What a microphone at the boards hears: the full top of the blade's noise.
            ("Close up", SkateParams { top_hz: 5500.0, blade_hz: 1000.0, grain: 0.6, ..Default::default() }),
            ("Goalie", SkateParams { stride_hz: 2.2, cut_level: 0.5, spray_level: 1.0, glide_level: 0.1, ..Default::default() }),
        ]
    }

    fn new(sr: f32) -> Self {
        Skate {
            sr,
            noise: Noise::new(0x71_0001),
            rng: Rng::new(crate::blocks::mix_seed(0x71_0002)),
            phase: 0.0,
            stride_pitch: 1.0,
            stride_power: 1.0,
            stride_pace: 1.0,
            blade: Svf::default(),
            top: Svf::default(),
            flutter: SlowNoise::new(0x71_0003),
            rough: SlowNoise::new(0x71_0004),
            sing: Svf::default(),
            hush: Svf::default(),
            spray: Svf::default(),
            spray_top: Svf::default(),
            chatter: SlowNoise::new(0x71_0005),
            chips: Dust::new(0x71_0006),
            chip_env: 0.0,
            chip_soft: 0.0,
            chip_bp: Svf::default(),
        }
    }

    fn block(&mut self, x: &[f32], p: &SkateParams, out: &mut [f32]) {
        let (sr, dt) = (self.sr, out.len() as f32 / self.sr);
        let (speed, push, edge, surface) = (x[0], x[1], x[2], x[3]);

        // Strides come faster at speed. A new one picks its own pitch and strength.
        self.phase += p.stride_hz * (0.7 + 0.6 * speed) * self.stride_pace * dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.stride_pitch = (0.18 * self.rng.next_bipolar()).exp2();
            self.stride_power = self.rng.range(0.75, 1.0);
            self.stride_pace = self.rng.range(0.85, 1.15);
        }
        // "shhk": the blade bites in, then shaves until the leg is spent.
        let ph = self.phase;
        let bite = (ph / 0.07).min(1.0);
        let shave = (1.0 - ph / 0.62).max(0.0);
        let stride = bite * bite * (3.0 - 2.0 * bite) * shave.powf(1.3) * self.stride_power;
        // The shaving is not even: its level flutters, more on rough ice.
        let flutter = 1.0 + p.grain * (0.55 * self.flutter.advance(38.0, dt) + 0.45 * surface * self.rough.advance(85.0, dt));
        let cut_gain = p.cut_level * push * speed.powf(0.7) * stride * flutter.max(0.0) * 6.0;
        self.blade.set(FilterMode::BandPass, p.blade_hz * (0.85 + 0.3 * speed) * self.stride_pitch, 0.12, sr);
        self.top.set(FilterMode::LowPass, p.top_hz, 0.1, sr);

        // Gliding: a faint metallic singing and a hush, both nearly nothing.
        self.sing.set(FilterMode::BandPass, p.sing_hz * (0.9 + 0.2 * speed), 0.96, sr);
        self.hush.set(FilterMode::LowPass, 600.0, 0.1, sr);
        let sing_gain = p.glide_level * speed * speed * (1.0 - 0.7 * edge) * (1.0 - 0.8 * push) * 0.07;
        let hush_gain = p.glide_level * speed * 0.3;

        // On the edges: a thick, dull spray of shaved ice, juddering as the blade chatters.
        let judder = 0.75 + 0.25 * self.chatter.advance(30.0, dt);
        self.spray.set(FilterMode::BandPass, 620.0 * (1.0 + 0.3 * speed), 0.15, sr);
        self.spray_top.set(FilterMode::LowPass, p.top_hz.min(2200.0), 0.1, sr);
        let spray_gain = p.spray_level * edge * speed.powf(0.5) * judder * 7.0;

        // Cut-up ice: the blade knocks through ruts. Dense and soft, a texture rather than ticks.
        let chip_p = 120.0 * surface * speed * (0.3 + 0.7 * push.max(edge)) / sr;
        self.chip_bp.set(FilterMode::BandPass, 520.0, 0.4, sr);
        let (chip_decay, chip_attack) = ((-1.0 / (0.008 * sr)).exp(), 1.0 - (-1.0 / (0.002 * sr)).exp());
        let chip_gain = surface * 5.0;

        for o in out.iter_mut() {
            let (w, pk) = (self.noise.white(), self.noise.pink());
            let mut y = self.top.tick(self.blade.tick(w)) * cut_gain;
            y += self.sing.tick(w) * sing_gain + self.hush.tick(pk) * hush_gain;
            if spray_gain > 0.0 {
                y += self.spray_top.tick(self.spray.tick(pk)) * spray_gain;
            }
            if chip_gain > 0.0 {
                let c = self.chips.tick(chip_p);
                if c > 0.0 {
                    self.chip_env = self.chip_env.max(c);
                }
                self.chip_soft += (self.chip_env - self.chip_soft) * chip_attack;
                y += self.chip_bp.tick(pk * self.chip_soft) * chip_gain;
                self.chip_env *= chip_decay;
            }
            *o = y * p.gain;
        }
    }
}
