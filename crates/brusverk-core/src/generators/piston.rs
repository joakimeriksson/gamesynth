//! Piston engine as a physical model.
//!
//! [`super::vehicles::Combustion`] is a cheap, generic engine: evenly spaced firings into one
//! filter. This one models where an engine's character actually comes from:
//!
//! * **Firing order and banks.** Each cylinder fires at its crank angle into its own bank's
//!   exhaust. A cross-plane V8 fires L R R L R L L R, so each pipe gets pulses 90, 180 and
//!   270 degrees apart; that uneven spacing is the V8 rumble (half-order harmonics). A
//!   flat-plane V8 fires each bank evenly and screams instead.
//! * **Pipes.** Header and exhaust are quarter-wave resonators (closed at the valve, open at
//!   the tail). Their formants stay put while the revs sweep through them, as on a real car.
//! * **Blowdown pulses.** A firing releases a pressure pulse whose length is set by gas
//!   dynamics at idle (distinct pops) and by crank angle at speed; load makes it steeper and
//!   the flow noisier.
//! * **Everything else you hear near an engine:** intake roar through the throttle, valvetrain
//!   ticking, block knock, a blower.
//! * **Heavy engines.** A turbocharger that spools with exhaust flow, lags behind the throttle
//!   and dumps its boost on a lift; an engine (compression-release) brake that barks on a closed
//!   throttle; diesel injection clatter; two-stroke operation (the firing order every turn).
//!
//! The banks go to the two channels (`stereo/width`), which is most of the fun on headphones.
//! Inputs, their order and the shared parameter names match `combustion`, so a game can switch
//! by changing the generator name.

use core::f32::consts::E;

use super::vehicles::{ENGINE_CYCLES, OFF_ON};
use crate::blocks::{hz_coef, mix_seed, settle_coef, DelayLine, OnePole, Phasor, BLOCK};
use crate::filter::{FilterMode, Svf};
use crate::math::{soft_clip, Rng};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::params::{exp, lin, ParamKind, GAIN, UNIT};

pub const PISTON_LAYOUTS: [&str; 8] = ["Crossplane V8", "Flat-plane V8", "Inline 4", "Boxer 4", "Inline 6", "V-twin", "Single", "V12"];

/// One firing: (crank angle as a fraction of the 720 degree cycle, exhaust bank).
type Firing = (f32, usize);

/// Firing order 1-8-4-3-6-5-7-2 with odd cylinders on the left bank.
const CROSSPLANE_V8: [Firing; 8] = [(0.0, 0), (0.125, 1), (0.25, 1), (0.375, 0), (0.5, 1), (0.625, 0), (0.75, 0), (0.875, 1)];
const FLATPLANE_V8: [Firing; 8] = [(0.0, 0), (0.125, 1), (0.25, 0), (0.375, 1), (0.5, 0), (0.625, 1), (0.75, 0), (0.875, 1)];
const INLINE_4: [Firing; 4] = [(0.0, 0), (0.25, 0), (0.5, 0), (0.75, 0)];
/// 1-3-2-4: two firings from one side, then two from the other.
const BOXER_4: [Firing; 4] = [(0.0, 0), (0.25, 0), (0.5, 1), (0.75, 1)];
const INLINE_6: [Firing; 6] = [(0.0, 0), (1.0 / 6.0, 0), (2.0 / 6.0, 0), (0.5, 0), (4.0 / 6.0, 0), (5.0 / 6.0, 0)];
/// 45 degree twin on one crank pin: 315 then 405 degrees.
const V_TWIN: [Firing; 2] = [(0.0, 0), (315.0 / 720.0, 1)];
const SINGLE: [Firing; 1] = [(0.0, 0)];
/// 60 degree V12: a firing every 60 degrees, banks alternating, each bank an even inline six.
const V12: [Firing; 12] = [(0.0, 0), (1.0 / 12.0, 1), (2.0 / 12.0, 0), (0.25, 1), (4.0 / 12.0, 0), (5.0 / 12.0, 1), (0.5, 0), (7.0 / 12.0, 1), (8.0 / 12.0, 0), (0.75, 1), (10.0 / 12.0, 0), (11.0 / 12.0, 1)];

fn layout(index: usize) -> &'static [Firing] {
    match index {
        0 => &CROSSPLANE_V8,
        1 => &FLATPLANE_V8,
        2 => &INLINE_4,
        3 => &BOXER_4,
        4 => &INLINE_6,
        5 => &V_TWIN,
        6 => &SINGLE,
        _ => &V12,
    }
}

model_params! {
    /// Parameters of the physical piston engine. Lengths are real pipe lengths in metres.
    ///
    /// The defaults are a stock cross-plane V8 heard from behind the car. Its combustion and
    /// exhaust values were fitted to a public-domain recording of a 3.5 litre Rover V8 at idle
    /// and at 3000 rpm (third-octave levels within about 4 dB, half-order "rumble" share 0.23
    /// and 0.94 against 0.21 and 0.92 measured); the "Stock V8" preset adds the engine-bay
    /// levels of valvetrain and fan from the same fit.
    PistonParams / PistonParamId {
        layout: "engine/layout" = 0.0, ParamKind::Enum(&PISTON_LAYOUTS);
        idle_rpm: "engine/idle_rpm" = 750.0, lin(300.0, 3000.0);
        max_rpm: "engine/max_rpm" = 6500.0, lin(2000.0, 16000.0);
        rev_up: "engine/rev_up" = 0.7, lin(0.05, 6.0);
        rev_down: "engine/rev_down" = 1.4, lin(0.05, 6.0);
        external_rpm: "engine/external_rpm" = 0.0, ParamKind::Enum(&OFF_ON);
        roughness: "engine/roughness" = 0.5, UNIT;
        lope: "engine/cam_lope" = 0.5, UNIT;
        limiter: "engine/rev_limiter" = 0.0, UNIT;
        pulse_deg: "pulse/degrees" = 25.0, lin(15.0, 180.0);
        turbulence: "pulse/turbulence" = 0.65, UNIT;
        steepening: "pulse/steepening" = 0.35, UNIT;
        header_m: "exhaust/header_m" = 0.75, lin(0.2, 2.0);
        length_m: "exhaust/length_m" = 2.2, lin(0.5, 6.0);
        resonance: "exhaust/resonance" = 0.375, lin(0.0, 0.92);
        muffling: "exhaust/muffling" = 0.19, UNIT;
        crossover: "exhaust/crossover" = 0.2, UNIT;
        unequal_ms: "exhaust/unequal_ms" = 2.4, lin(0.0, 6.0);
        interference: "exhaust/interference" = 0.8, UNIT;
        drive: "exhaust/drive" = 0.35, UNIT;
        burble: "exhaust/overrun_burble" = 0.4, UNIT;
        backfire: "exhaust/backfire" = 0.0, UNIT;
        intake_level: "intake/level" = 0.4, UNIT;
        intake_hz: "intake/hz" = 570.0, exp(120.0, 2000.0);
        valvetrain: "mechanical/valvetrain" = 0.3, UNIT;
        block: "mechanical/block" = 0.3, UNIT;
        fan: "mechanical/fan" = 0.25, UNIT;
        blower_level: "blower/level" = 0.0, UNIT;
        blower_ratio: "blower/ratio" = 14.0, lin(4.0, 40.0);
        boost_drive: "boost/drive" = 0.5, UNIT;
        wear: "damage/wear" = 0.0, UNIT;
        misfire: "damage/misfire" = 0.6, UNIT;
        rattle: "damage/rattle" = 0.5, UNIT;
        leak: "damage/exhaust_leak" = 0.5, UNIT;
        cycle: "engine/cycle" = 0.0, ParamKind::Enum(&ENGINE_CYCLES);
        jake: "engine/jake_brake" = 0.0, UNIT;
        clatter: "mechanical/clatter" = 0.0, UNIT;
        turbo_level: "turbo/level" = 0.0, UNIT;
        turbo_hz: "turbo/hz" = 5200.0, exp(1500.0, 12000.0);
        turbo_lag: "turbo/lag_s" = 0.8, lin(0.1, 4.0);
        blowoff: "turbo/blowoff" = 0.5, UNIT;
        width: "stereo/width" = 0.6, UNIT;
        gain: "master/gain" = 0.9, GAIN;
    }
}

/// Speed of sound in hot exhaust gas, m/s.
const C_EXHAUST: f32 = 520.0;
/// Fixed per-cylinder breathing imbalance of a lumpy cam.
const LOPE_LEVEL: [f32; 12] = [0.6, -0.4, 1.0, -0.7, 0.2, 0.8, -0.5, 0.3, -0.9, 0.5, -0.2, 0.7];

pub struct Piston {
    sr: f32,
    /// Smoothed inputs, in `INPUTS` order.
    x: [f32; 5],
    rev: f32,
    layout: usize,
    /// Position in the 720 degree cycle, 0..1.
    phase: f32,
    /// Index of the next firing in the layout table.
    next: usize,
    /// Cycle-to-cycle speed variation until the next firing.
    timing: f32,
    cyl_gain: [f32; 12],
    /// Relative primary-tube length per cylinder, 0..1.
    header_spread: [f32; 12],
    rng: Rng,
    noise: Noise,
    /// Firings on their way down the headers, per bank (a ring of future samples).
    ring: [Vec<f32>; 2],
    ring_pos: usize,
    pulse: [[f32; 2]; 2],
    prev_flow: [f32; 2],
    header: [DelayLine; 2],
    header_loss: [OnePole; 2],
    pipe: [DelayLine; 2],
    pipe_loss: [OnePole; 2],
    /// Two stages per bank: a real muffler cuts steeply.
    muffler: [[Svf; 2]; 2],
    dc: [OnePole; 2],
    /// Noise burst of a backfire or overrun pop, per bank.
    bang: [f32; 2],
    intake_env: [f32; 2],
    intake: Svf,
    /// Samples until the valve tick between two firings.
    tick_wait: f32,
    valve: [Svf; 2],
    knock: [Svf; 2],
    fan: Svf,
    leak: Svf,
    rattle: Svf,
    rattle_env: f32,
    blower: [Phasor; 2],
    thr_slow: f32,
    backfire_cooldown: f32,
    pops_left: u32,
    pop_wait: f32,
    limiter_phase: f32,
    /// Seconds the throttle has been closed (the engine brake waits out gear changes).
    closed_secs: f32,
    /// Turbocharger speed 0..1.
    spool: f32,
    turbo_phase: Phasor,
    turbo_bp: Svf,
    /// Blow-off burst after a lift, and its flutter.
    bov: f32,
    bov_cooldown: f32,
    bov_phase: Phasor,
    bov_bp: Svf,
    clatter: [Svf; 2],
}

impl Piston {
    #[allow(clippy::too_many_lines)]
    fn render(&mut self, x_in: &[f32], p: &PistonParams, width: f32, left: &mut [f32], right: &mut [f32]) {
        let sr = self.sr;
        let dt = left.len() as f32 / sr;
        for (k, (sm, target)) in self.x.iter_mut().zip(x_in).enumerate() {
            *sm += (*target - *sm) * settle_coef(if k == 2 { 0.012 } else { 0.05 }, dt);
        }
        let [throttle, load, rpm_in, damage_in, boost] = self.x;
        let external = p.external_rpm >= 0.5;
        if external {
            self.rev = rpm_in;
        } else {
            let secs = if throttle > self.rev { p.rev_up } else { p.rev_down };
            self.rev += (throttle - self.rev) * settle_coef(secs, dt);
        }
        let rev = self.rev;
        let damage = (damage_in + p.wear).min(1.0);

        let li = (p.layout.round() as usize).min(PISTON_LAYOUTS.len() - 1);
        let fir = layout(li);
        let ncyl = fir.len();
        if li != self.layout {
            self.layout = li;
            self.next = fir.iter().position(|f| f.0 > self.phase).unwrap_or(ncyl);
        }
        let two_banks = fir.iter().any(|f| f.1 == 1);

        let rpm = p.idle_rpm + (p.max_rpm - p.idle_rpm) * rev;
        // Crank speed as four-stroke cycles per second; a two-stroke runs its firing order every turn.
        let cycle_hz = rpm / 120.0;
        let step = cycle_hz * if p.cycle >= 0.5 { 2.0 } else { 1.0 } / sr;

        // Engine brake: after a moment on a closed throttle the exhaust valves release each
        // cylinder's compression into the pipe, a bark far louder than coasting.
        self.closed_secs = if throttle < 0.08 { self.closed_secs + dt } else { 0.0 };
        let jake = p.jake * ((self.closed_secs - 0.15) / 0.15).clamp(0.0, 1.0) * ((rev - 0.12) / 0.1).clamp(0.0, 1.0);

        // Blowdown pulse: a critically damped two-pole response. At speed its length is a
        // crank angle; at idle gas dynamics end it after a couple of milliseconds, which is
        // why an idling engine pops instead of humming. Load makes it shorter and stronger.
        let tau = ((p.pulse_deg / 720.0) / cycle_hz / 3.0).min(0.0022 - 0.0012 * throttle) * (1.0 - 0.45 * jake);
        let a = 1.0 - (-1.0 / (tau * sr)).exp();
        let inject = E / a;
        let strength = 0.35 + 0.65 * throttle.powf(0.8);
        let variation = p.roughness * (0.1 + 0.25 * (1.0 - throttle));
        let lumpy = p.lope * (1.0 - rev).max(0.0).powf(1.5) * (1.0 - 0.6 * throttle);
        let overrun = (rev - throttle - 0.15).max(0.0) * p.burble;
        let steep = p.steepening * (0.3 + 0.7 * throttle.max(jake)) * 1.5 * tau * sr;
        let turb = p.turbulence * 1.5 * (1.0 + jake);

        // Lift-off backfire, rev limiter, damage: as in `combustion`.
        self.thr_slow += (throttle - self.thr_slow) * settle_coef(0.2, dt);
        self.backfire_cooldown = (self.backfire_cooldown - dt).max(0.0);
        if p.backfire > 0.0 && self.pops_left == 0 && self.backfire_cooldown == 0.0 && self.thr_slow - throttle > 0.3 && rev > 0.35 {
            self.backfire_cooldown = 0.7;
            if self.rng.chance((p.backfire * (0.7 + 0.6 * damage)).min(1.0)) {
                self.pops_left = 1 + self.rng.next_u32() % 3;
                self.pop_wait = self.rng.range(0.0, 0.05);
            }
        }
        let limiting = external && p.limiter > 0.0 && rev > 0.97 && throttle > 0.6;
        if limiting {
            self.limiter_phase = (self.limiter_phase + dt * (9.0 + 5.0 * self.rng.next_f32())).fract();
        }
        let cut = limiting && self.limiter_phase < 0.25 + 0.45 * p.limiter;
        let misfire_p = damage.powf(1.5) * 0.35 * p.misfire;
        let dead = ((damage - 0.5) * 2.0).clamp(0.0, 1.0) * p.misfire;
        let rattle_p = damage * p.rattle * 0.6;
        if rattle_p > 0.0 {
            self.rattle.set(FilterMode::BandPass, 2600.0 * (0.6 * self.rng.next_bipolar()).exp2(), 0.9, sr);
        }
        let leak_gain = damage * p.leak * 0.5;
        if leak_gain > 0.0 {
            self.leak.set(FilterMode::HighPass, 2500.0, 0.3, sr);
        }
        let backfire_gain = 2.2 * (0.6 + 0.8 * damage) * p.backfire.sqrt();

        // Turbocharger: the turbine spools with exhaust flow and lags behind the throttle.
        let (mut whistle_gain, mut whistle_inc, mut bov_gain) = (0.0, 0.0, 0.0);
        if p.turbo_level > 0.0 {
            let target = (throttle * (0.25 + 0.75 * rev) + 0.3 * boost).min(1.0);
            // Lifting off a spooled turbo dumps the boost through the blow-off valve.
            self.bov_cooldown = (self.bov_cooldown - dt).max(0.0);
            if p.blowoff > 0.0 && self.bov_cooldown == 0.0 && self.thr_slow - throttle > 0.25 && self.spool > 0.3 {
                self.bov = self.spool;
                self.bov_cooldown = 0.6;
            }
            let secs = if target > self.spool { p.turbo_lag } else { p.turbo_lag * 1.5 };
            self.spool += (target - self.spool) * settle_coef(secs, dt);
            let hz = (p.turbo_hz * (0.3 + 0.7 * self.spool)).min(sr * 0.4);
            self.turbo_bp.set(FilterMode::BandPass, hz, 0.97, sr);
            self.bov_bp.set(FilterMode::BandPass, 2600.0, 0.2, sr);
            whistle_inc = hz / sr;
            whistle_gain = p.turbo_level * self.spool.powf(1.5) * 0.2;
            bov_gain = p.blowoff * p.turbo_level.sqrt() * 1.5;
        }
        let bov_decay = (-1.0 / (0.13 * sr)).exp();

        // Exhaust: header and pipe as quarter-wave resonators, then the muffler.
        let unequal = p.unequal_ms * 0.001 * sr;
        let header_d = 2.0 * p.header_m / C_EXHAUST * sr;
        let pipe_d = [2.0 * p.length_m / C_EXHAUST * sr * 0.97, 2.0 * p.length_m / C_EXHAUST * sr * 1.03];
        let (g_h, g_p) = (0.25 * p.resonance, p.resonance);
        let (loss_h, loss_p) = (hz_coef(2500.0, sr), hz_coef(900.0 + 1400.0 * (1.0 - p.muffling), sr));
        let muffler_hz = 6000.0 * (160.0f32 / 6000.0).powf(p.muffling) * (0.8 + 0.6 * throttle);
        for m in self.muffler.iter_mut().flatten() {
            m.set(FilterMode::LowPass, muffler_hz, 0.15, sr);
        }
        let dc_coef = hz_coef(22.0, sr);
        let pipe_norm = (1.0 - 0.5 * g_p) * (1.0 - 0.3 * g_h);
        let cross = if two_banks { p.crossover } else { 1.0 };
        let drive = 1.0 + p.drive * 4.0 + boost * p.boost_drive * 4.0;
        let drive_norm = 1.0 / (1.0 + 0.2 * (drive - 1.0));
        let bang_decay = (-1.0 / (0.03 * sr)).exp();

        // Intake: noise through the throttle, breathing with the valves, in the manifold's resonance.
        let tau_i = ((110.0 / 720.0) / cycle_hz / 3.0).min(0.004);
        let ai = 1.0 - (-1.0 / (tau_i * sr)).exp();
        self.intake.set(FilterMode::BandPass, p.intake_hz, 0.55, sr);
        let intake_gain = p.intake_level * (0.06 + 0.94 * throttle.powf(1.5)) * (0.4 + 0.6 * rev) * (1.0 + boost) * 1.2;

        // Mechanical: valves closing ring the head, combustion knocks the block.
        self.valve[0].set(FilterMode::BandPass, 2300.0, 0.9, sr);
        self.valve[1].set(FilterMode::BandPass, 3900.0, 0.88, sr);
        self.knock[0].set(FilterMode::BandPass, 820.0, 0.8, sr);
        self.knock[1].set(FilterMode::BandPass, 1650.0, 0.75, sr);
        let valve_gain = p.valvetrain * (0.5 + 0.5 * rev) * 0.5;
        let knock_gain = p.block * (0.25 + 0.75 * throttle) * 0.25;
        let tick_gap = 0.5 / ncyl as f32 / step;
        // Diesel injection clatter: a hard tick per firing, most obvious at idle.
        let clatter_gain = p.clatter * (1.0 - 0.5 * rev) * 1.6;
        if clatter_gain > 0.0 {
            self.clatter[0].set(FilterMode::BandPass, 1150.0, 0.85, sr);
            self.clatter[1].set(FilterMode::BandPass, 2700.0, 0.8, sr);
        }
        // Cooling fan, belts and air rushing through the bay: broadband, rising fast with revs.
        self.fan.set(FilterMode::BandPass, 1800.0 + 2500.0 * rev, 0.0, sr);
        let fan_gain = p.fan * (0.1 + 0.9 * rev.powf(1.3)) * 1.6;
        let rattle_decay = (-1.0 / (0.004 * sr)).exp();

        let blower_gain = p.blower_level * (0.25 + 0.4 * throttle + 0.6 * boost) * (0.3 + 0.7 * rev) * 0.12;
        let blower_inc = rpm / 60.0 * p.blower_ratio / sr;

        let level = (0.5 + 0.5 * load.max(throttle * 0.5)) * (0.7 + 0.3 * rev) * p.gain * (1.0 + 0.25 * boost) * 1.5 * (1.0 + 0.5 * jake);
        let (wa, wb) = (0.5 * (1.0 + width), 0.5 * (1.0 - width));
        let ring_len = self.ring[0].len();

        for (ol, or) in left.iter_mut().zip(right.iter_mut()) {
            // ---- crank: fire every cylinder whose angle was passed during this sample ----
            let moved = step * self.timing;
            let mut ph = self.phase + moved;
            if ph >= 1.0 {
                ph -= 1.0;
                self.next = 0;
            }
            let (mut knock_in, mut tick_in, mut intake_in, mut clatter_in) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            while self.next < ncyl && ph >= fir[self.next].0 {
                let cyl = self.next;
                let (angle, bank) = fir[cyl];
                self.next += 1;
                let past = ((ph - angle) / moved).clamp(0.0, 1.0);
                let mut amp = self.cyl_gain[cyl] * (1.0 + variation * self.rng.next_bipolar());
                // No two cycles take exactly as long.
                self.timing = 1.0 + (0.01 + 0.05 * p.roughness) * self.rng.next_bipolar();
                if lumpy > 0.0 {
                    amp *= 1.0 + 0.45 * lumpy * LOPE_LEVEL[cyl];
                    if self.rng.chance(0.12 * lumpy) {
                        // A weak fire, and the crank sags until the next one.
                        amp *= 0.55;
                        self.timing *= 1.0 - 0.06 * lumpy;
                    }
                }
                // Back-pressure: a cylinder that fires into a bank still loaded by the previous
                // pulse blows down less. This is what keeps the two banks from summing to an
                // even pulse train, so the rumble survives a mono mix.
                amp *= 1.0 - p.interference * 0.6 * self.pulse[bank][1].clamp(0.0, 1.0);
                if cut || (misfire_p > 0.0 && self.rng.chance(misfire_p)) {
                    amp = 0.0;
                }
                if dead > 0.0 && cyl == ncyl - 1 && ncyl > 1 {
                    amp *= 1.0 - dead;
                }
                let mut fire = amp * strength;
                if jake > 0.0 {
                    fire = fire.max(self.cyl_gain[cyl] * jake * (0.75 + 0.5 * rev));
                }
                if clatter_gain > 0.0 {
                    clatter_in += self.rng.range(0.6, 1.0);
                }
                if overrun > 0.0 && self.rng.chance((overrun * 0.6).min(0.7)) {
                    // Unburnt fuel lighting in the pipe: a pop that owes nothing to the throttle.
                    fire = self.rng.range(0.5, 1.1);
                    self.bang[bank] = self.bang[bank].max(0.5);
                }
                // Down its primary tube: unequal lengths (and, with one collector, a whole
                // bank arriving late) shift each pulse in time.
                let t = 1.0 - past + unequal * (0.5 * self.header_spread[cyl] + bank as f32);
                let (i0, f) = (t as usize, t.fract());
                let slot = (self.ring_pos + i0) % ring_len;
                self.ring[bank][slot] += fire * (1.0 - f);
                self.ring[bank][(slot + 1) % ring_len] += fire * f;
                knock_in += fire;
                intake_in += 1.0;
                tick_in += self.rng.range(0.5, 1.0);
                self.tick_wait = tick_gap;
                if rattle_p > 0.0 && self.rng.chance(rattle_p) {
                    self.rattle_env = self.rattle_env.max(self.rng.range(0.4, 1.0));
                }
            }
            self.phase = ph;
            if self.tick_wait > 0.0 {
                self.tick_wait -= 1.0;
                if self.tick_wait <= 0.0 {
                    tick_in += self.rng.range(0.4, 0.9);
                }
            }
            if self.pops_left > 0 {
                self.pop_wait -= 1.0 / sr;
                if self.pop_wait <= 0.0 {
                    let bank = if two_banks { (self.rng.next_u32() & 1) as usize } else { 0 };
                    self.ring[bank][self.ring_pos] += backfire_gain;
                    self.bang[bank] = 1.0;
                    self.pops_left -= 1;
                    self.pop_wait = self.rng.range(0.04, 0.16);
                }
            }

            // ---- exhaust: pulses -> flow -> header -> pipe -> muffler, per bank ----
            let mut src = [0.0f32; 2];
            for b in 0..2 {
                let e = core::mem::take(&mut self.ring[b][self.ring_pos]);
                let [y1, y2] = &mut self.pulse[b];
                *y1 += a * (e * inject - *y1);
                *y2 += a * (*y1 - *y2);
            }
            self.ring_pos = (self.ring_pos + 1) % ring_len;
            for (b, s) in src.iter_mut().enumerate() {
                // A single bank feeds both pipes.
                let flow = self.pulse[if two_banks { b } else { 0 }][1];
                let wn = self.noise.white();
                // Strong pulses steepen on their way down the pipe; fast gas is turbulent.
                *s = flow + steep * (flow - self.prev_flow[b]) + (flow * turb + self.bang[b] * backfire_gain.max(1.0)) * wn;
                self.prev_flow[b] = flow;
                self.bang[b] *= bang_decay;
            }
            let mid = 0.5 * (src[0] + src[1]);
            let mut tail = [0.0f32; 2];
            for b in 0..2 {
                let x0 = src[b] + cross * (mid - src[b]);
                let h = x0 - g_h * self.header_loss[b].lp(self.header[b].read(header_d), loss_h);
                self.header[b].write(h);
                let z = h - g_p * self.pipe_loss[b].lp(self.pipe[b].read(pipe_d[b]), loss_p);
                self.pipe[b].write(z);
                let [m0, m1] = &mut self.muffler[b];
                let out = self.dc[b].hp(m1.tick(m0.tick(z * pipe_norm)), dc_coef);
                tail[b] = soft_clip(out * drive) * drive_norm;
            }

            // ---- everything that is not the exhaust ----
            let [i1, i2] = &mut self.intake_env;
            *i1 += ai * (intake_in * E / ai - *i1);
            *i2 += ai * (*i1 - *i2);
            let breathing = *i2;
            let wn = self.noise.white();
            let mut shared = self.intake.tick(self.noise.pink() * (0.25 + 1.5 * breathing)) * intake_gain
                + (self.valve[0].tick(tick_in) + 0.7 * self.valve[1].tick(tick_in)) * valve_gain
                + (self.knock[0].tick(knock_in) + 0.6 * self.knock[1].tick(knock_in)) * knock_gain
                + self.fan.tick(wn) * fan_gain;
            if rattle_p > 0.0 || self.rattle_env > 1e-4 {
                shared += self.rattle.tick(wn * self.rattle_env) * 1.5;
                self.rattle_env *= rattle_decay;
            }
            if leak_gain > 0.0 {
                shared += self.leak.tick(wn) * self.pulse[0][1].min(2.0) * leak_gain;
            }
            if blower_gain > 0.0 {
                self.blower[0].tick(blower_inc);
                self.blower[1].tick(blower_inc * 2.0);
                shared += (self.blower[0].sin() + 0.45 * self.blower[1].sin()) * blower_gain;
            }
            if clatter_gain > 0.0 {
                shared += (self.clatter[0].tick(clatter_in) + 0.7 * self.clatter[1].tick(clatter_in)) * clatter_gain;
            }
            if whistle_gain > 0.0 || self.bov > 1e-4 {
                self.turbo_phase.tick(whistle_inc);
                shared += (0.5 * self.turbo_phase.sin() + 0.6 * self.turbo_bp.tick(wn)) * whistle_gain;
                if self.bov > 1e-4 {
                    // The dumped boost flutters as the compressor surges.
                    self.bov_phase.tick(24.0 / sr);
                    shared += self.bov_bp.tick(wn) * self.bov * (0.6 + 0.4 * self.bov_phase.sin()) * bov_gain;
                    self.bov *= bov_decay;
                }
            }
            // Summed pipes first, so that at width 0 both channels are the same number.
            *ol = (shared + (wa * tail[0] + wb * tail[1])) * level;
            *or = (shared + (wa * tail[1] + wb * tail[0])) * level;
        }
    }
}

impl Generator for Piston {
    type P = PistonParams;
    const NAME: &'static str = "piston";
    const CATEGORY: &'static str = "vehicles";
    const DOC: &'static str = "Piston engine, physical model: firing order into two exhaust banks, pipe resonances, intake, valvetrain. Stereo.";
    const INPUTS: &'static [InputSpec] = &[
        InputSpec { name: "throttle", default: 0.0, doc: "How hard it burns; also drives the revs (with inertia) unless engine/external_rpm is on" },
        InputSpec { name: "load", default: 0.3, doc: "Engine load: louder and fuller" },
        InputSpec { name: "rpm", default: 0.0, doc: "Revs 0..1 between idle_rpm and max_rpm, used when engine/external_rpm is on (gears, limiter, clutch)" },
        InputSpec { name: "damage", default: 0.0, doc: "Wrecked engine: misfires, a dead cylinder, rattle, exhaust-leak hiss, louder backfires" },
        InputSpec { name: "boost", default: 0.0, doc: "Nitro / blower boost: supercharger whine and extra drive" },
    ];
    // Inputs are smoothed in `render` (12 ms for a game-driven rpm, 50 ms for the rest).
    const INPUT_SMOOTH_SECS: f32 = 0.0;

    fn presets() -> Vec<(&'static str, PistonParams)> {
        let d = PistonParams::default();
        vec![
            // The fit to the recording, microphone in the engine bay.
            ("Stock V8", PistonParams { idle_rpm: 835.0, roughness: 0.65, lope: 0.65, valvetrain: 0.65, fan: 0.69, ..d }),
            ("Muscle V8", PistonParams {
                idle_rpm: 800.0, max_rpm: 6200.0, lope: 0.85, roughness: 0.6, length_m: 1.7, muffling: 0.06, resonance: 0.5, crossover: 0.05, steepening: 0.6, burble: 0.7,
                backfire: 0.4, limiter: 0.5, ..d
            }),
            ("Blown V8", PistonParams {
                idle_rpm: 850.0, max_rpm: 6800.0, lope: 0.8, roughness: 0.6, header_m: 0.5, length_m: 0.9, muffling: 0.02, resonance: 0.5, crossover: 0.0, steepening: 0.7,
                turbulence: 0.8, burble: 0.8, backfire: 0.5, limiter: 0.6, blower_level: 0.55, drive: 0.5, ..d
            }),
            ("Flat-plane V8", PistonParams {
                layout: 1.0, idle_rpm: 1000.0, max_rpm: 8500.0, rev_up: 0.4, rev_down: 0.8, lope: 0.1, roughness: 0.3, length_m: 1.8, header_m: 0.6, muffling: 0.12, crossover: 0.1,
                unequal_ms: 0.4, limiter: 0.6, ..d
            }),
            ("Boxer rumble", PistonParams { layout: 3.0, idle_rpm: 800.0, max_rpm: 7000.0, lope: 0.15, roughness: 0.35, crossover: 1.0, unequal_ms: 2.6, length_m: 2.4, muffling: 0.3, limiter: 0.5, ..d }),
            ("Inline four", PistonParams { layout: 2.0, idle_rpm: 850.0, max_rpm: 7500.0, rev_up: 0.5, rev_down: 1.0, lope: 0.05, roughness: 0.3, unequal_ms: 0.4, length_m: 2.4, muffling: 0.4, intake_level: 0.5, ..d }),
            ("Diesel six", PistonParams {
                layout: 4.0, idle_rpm: 600.0, max_rpm: 2600.0, rev_up: 2.0, rev_down: 2.6, lope: 0.0, roughness: 0.25, pulse_deg: 40.0, unequal_ms: 0.6, length_m: 4.2, muffling: 0.45,
                burble: 0.0, block: 0.9, valvetrain: 0.5, intake_level: 0.3, clatter: 0.5, turbo_level: 0.3, gain: 0.55, ..d
            }),
            // A big-rig inline six: turbo whistle and blow-off, engine brake on a closed throttle.
            ("Heavy truck", PistonParams {
                layout: 4.0, idle_rpm: 550.0, max_rpm: 2200.0, rev_up: 2.4, rev_down: 3.0, lope: 0.0, roughness: 0.3, pulse_deg: 45.0, unequal_ms: 0.8, header_m: 1.2, length_m: 4.5,
                resonance: 0.5, muffling: 0.3, burble: 0.0, block: 0.8, valvetrain: 0.4, clatter: 0.7, intake_level: 0.35, fan: 0.4, turbo_level: 0.6, turbo_hz: 4200.0, turbo_lag: 1.2,
                blowoff: 0.5, jake: 0.9, drive: 0.45, gain: 0.5, ..d
            }),
            // A huge diesel V8 on open stacks, built to be heard coming.
            ("War rig V8", PistonParams {
                idle_rpm: 520.0, max_rpm: 3000.0, rev_up: 1.6, rev_down: 2.2, lope: 0.7, roughness: 0.6, pulse_deg: 35.0, header_m: 1.0, length_m: 3.4, resonance: 0.55, muffling: 0.04,
                crossover: 0.0, steepening: 0.7, turbulence: 0.8, burble: 0.3, backfire: 0.4, limiter: 0.4, block: 0.7, clatter: 0.5, turbo_level: 0.5, turbo_hz: 3800.0, turbo_lag: 1.0,
                blowoff: 0.7, jake: 1.0, drive: 0.6, gain: 0.55, ..d
            }),
            // Blown two-stroke V8 diesel: fires every turn, so it sounds revved twice as high.
            ("Two-stroke diesel V8", PistonParams {
                cycle: 1.0, idle_rpm: 500.0, max_rpm: 2300.0, rev_up: 1.2, rev_down: 1.8, lope: 0.2, roughness: 0.4, pulse_deg: 50.0, header_m: 0.8, length_m: 3.0, muffling: 0.1,
                crossover: 0.3, burble: 0.0, block: 0.6, clatter: 0.5, blower_level: 0.6, blower_ratio: 12.0, jake: 0.8, gain: 0.5, ..d
            }),
            ("Tank V12", PistonParams {
                layout: 7.0, idle_rpm: 500.0, max_rpm: 2400.0, rev_up: 2.5, rev_down: 3.2, lope: 0.1, roughness: 0.35, pulse_deg: 45.0, header_m: 1.0, length_m: 2.6, muffling: 0.12,
                crossover: 0.0, burble: 0.0, block: 0.9, valvetrain: 0.6, clatter: 0.6, fan: 0.6, drive: 0.5, gain: 0.5, ..d
            }),
            ("V-twin", PistonParams {
                layout: 5.0, idle_rpm: 900.0, max_rpm: 5600.0, lope: 0.25, roughness: 0.4, pulse_deg: 60.0, header_m: 0.5, length_m: 1.2, muffling: 0.08, resonance: 0.5, crossover: 0.0,
                unequal_ms: 0.3, burble: 0.6, valvetrain: 0.45, ..d
            }),
            ("Thumper", PistonParams {
                layout: 6.0, idle_rpm: 1300.0, max_rpm: 8500.0, rev_up: 0.3, rev_down: 0.6, lope: 0.0, roughness: 0.35, header_m: 0.4, length_m: 1.0, muffling: 0.12, resonance: 0.5,
                burble: 0.5, limiter: 0.6, ..d
            }),
        ]
    }

    fn new(sr: f32) -> Self {
        let mut rng = Rng::new(mix_seed(0x46_0001));
        let mut cyl_gain = [1.0f32; 12];
        cyl_gain.iter_mut().for_each(|g| *g = rng.range(0.7, 1.0));
        let mut header_spread = [0.0f32; 12];
        header_spread.iter_mut().for_each(|h| *h = rng.next_f32());
        // Headers can delay a firing by up to 6 ms * 1.5; pipes are at most 6 m (plus 3%).
        let ring = || vec![0.0f32; (0.012 * sr) as usize + 8];
        let pipe = || DelayLine::new((2.0 * 6.2 / C_EXHAUST * sr) as usize + 8);
        let header = || DelayLine::new((2.0 * 2.0 / C_EXHAUST * sr) as usize + 8);
        Piston {
            sr,
            x: [0.0, 0.3, 0.0, 0.0, 0.0],
            rev: 0.0,
            layout: 0,
            phase: 0.0,
            next: 1,
            timing: 1.0,
            cyl_gain,
            header_spread,
            rng,
            noise: Noise::new(mix_seed(0x46_0002)),
            ring: [ring(), ring()],
            ring_pos: 0,
            pulse: [[0.0; 2]; 2],
            prev_flow: [0.0; 2],
            header: [header(), header()],
            header_loss: [OnePole::default(); 2],
            pipe: [pipe(), pipe()],
            pipe_loss: [OnePole::default(); 2],
            muffler: [[Svf::default(); 2]; 2],
            dc: [OnePole::default(); 2],
            bang: [0.0; 2],
            intake_env: [0.0; 2],
            intake: Svf::default(),
            tick_wait: 0.0,
            valve: [Svf::default(); 2],
            knock: [Svf::default(); 2],
            fan: Svf::default(),
            leak: Svf::default(),
            rattle: Svf::default(),
            rattle_env: 0.0,
            blower: [Phasor::default(); 2],
            thr_slow: 0.0,
            backfire_cooldown: 0.0,
            pops_left: 0,
            pop_wait: 0.0,
            limiter_phase: 0.0,
            closed_secs: 0.0,
            spool: 0.0,
            turbo_phase: Phasor::default(),
            turbo_bp: Svf::default(),
            bov: 0.0,
            bov_cooldown: 0.0,
            bov_phase: Phasor::default(),
            bov_bp: Svf::default(),
            clatter: [Svf::default(); 2],
        }
    }

    fn snap(&mut self, x: &[f32], p: &PistonParams) {
        self.x.copy_from_slice(x);
        self.rev = if p.external_rpm >= 0.5 { x[2] } else { x[0] };
        self.thr_slow = x[0];
        self.spool = (x[0] * (0.25 + 0.75 * self.rev)).min(1.0);
        self.closed_secs = 0.0;
        self.bov = 0.0;
    }

    fn rpm(&self) -> Option<f32> {
        Some(self.rev)
    }

    /// The mono render is the stereo one at width zero: both pipes in both channels.
    fn block(&mut self, x: &[f32], p: &PistonParams, out: &mut [f32]) {
        let mut right = [0.0f32; BLOCK];
        let n = out.len();
        self.render(x, p, 0.0, out, &mut right[..n]);
    }

    fn block_stereo(&mut self, x: &[f32], p: &PistonParams, left: &mut [f32], right: &mut [f32]) {
        self.render(x, p, p.width, left, right);
    }
}
