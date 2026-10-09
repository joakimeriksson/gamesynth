//! UI and game-feel sounds: hover, click, toggles, confirm, cancel, error, notification, panels,
//! slider ticks, typing blips, coins, pickups, combos, level-up, countdown and a fanfare.
//!
//! Every event is a short score written in **chord tones and scale steps, not Hz**. The kit turns
//! the score into pitches through `music/key` and `music/scale`, so a whole game's UI is in tune
//! with itself and with its music, and game state moves along the scale:
//!
//! * `combo` raises any event by one scale step per 1/16 (a match-3 chain rises up the scale);
//! * `ui_slider` follows `value`, snapped to the scale;
//! * `ui_coin` scatters `count` coins into a quick arpeggio.
//!
//! `kit/style` decides what a note is (a soft sine, a marimba bar, struck glass, an 8-bit pulse, a
//! sci-fi FM chirp), so one parameter re-skins every event; the presets are those five styles.
//!
//! Each event is its own generator (`ui_click`, `ui_confirm`, …) on one shared parameter table.
//! A Godot player plays one stream, and what changes per play travels as start inputs (an event
//! selector as a parameter would race between players sharing the resource), so named events fit
//! that model; the shared table and preset names make the re-skin one loop in the game.
//!
//! Unlike the `fx` events, retriggering does not cut what is still ringing: notes go to a small
//! voice pool, so one player can run a whole combo chain or a burst of typing blips.

use core::marker::PhantomData;

use crate::blocks::{hz_coef, mix_seed, OnePole, Reverb, BLOCK};
use crate::filter::{FilterMode, Svf};
use crate::math::{midi_to_hz, Rng};
use crate::model::{Generator, InputSpec};
use crate::noise::Noise;
use crate::osc::{Oscillator, Waveform};
use crate::params::{exp, int, lin, ParamKind, GAIN, UNIT};

pub const STYLE_NAMES: [&str; 5] = ["Soft", "Wooden", "Glass", "Retro", "Sci-fi"];
pub const KEY_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
pub const SCALE_NAMES: [&str; 6] = ["Major", "Minor", "Pentatonic", "Minor pentatonic", "Dorian", "Lydian"];

/// Semitones of each scale above its tonic.
pub const SCALES: [&[i32]; 6] = [
    &[0, 2, 4, 5, 7, 9, 11],
    &[0, 2, 3, 5, 7, 8, 10],
    &[0, 2, 4, 7, 9],
    &[0, 3, 5, 7, 10],
    &[0, 2, 3, 5, 7, 9, 10],
    &[0, 2, 4, 6, 7, 9, 11],
];

/// Scale steps of the root, third and fifth in each scale (all six contain a triad on the tonic).
const TRIADS: [[i32; 3]; 6] = [[0, 2, 4], [0, 2, 4], [0, 2, 3], [0, 1, 3], [0, 2, 4], [0, 2, 4]];

/// The `combo` and `count` inputs carry whole numbers as n / 16 (inputs are 0..1).
pub const STEPS_PER_UNIT: f32 = 16.0;
/// Most coins one `ui_coin` trigger scatters.
pub const MAX_COINS: usize = 16;
const MAX_VOICES: usize = 24;
const MAX_NOTES: usize = 16;
/// Notes above this fold down an octave (a long combo on a high event stays pleasant).
const MAX_HZ: f32 = 5000.0;
/// Envelopes run to this level and are offset by it, so every note ends exactly at zero.
const EPS: f32 = 1e-3;
/// The reverb tail fades out over its last 50 ms instead of stopping.
const TAIL_FADE: f32 = 0.05;
/// Time the peak meter needs after the (already faded) end, see `Native::is_finished`.
const SETTLE: f32 = 0.15;
const COIN_GAP: f32 = 0.042;
const REF_HZ: f32 = 523.25;
/// Band-passed white noise sits far below its input; this brings a strike of 1 near a full tone.
const CLICK_GAIN: f32 = 6.0;

model_params! {
    /// Shared by every UI event. `kit/style` is the re-skin; `music/*` put the kit in the game's
    /// key; the `shape/*` and `space/*` macros bend a style (0.5, or 1.0 for length and speed, is
    /// the style as designed).
    UiParams / UiParamId {
        style: "kit/style" = 0.0, ParamKind::Enum(&STYLE_NAMES);
        key: "music/key" = 0.0, ParamKind::Enum(&KEY_NAMES);
        scale: "music/scale" = 0.0, ParamKind::Enum(&SCALE_NAMES);
        octave: "music/octave" = 0.0, int(-2, 2);
        tuning: "music/tuning_cents" = 0.0, lin(-50.0, 50.0);
        combo_max: "music/combo_max_steps" = 14.0, int(0, 16);
        slider_steps: "music/slider_steps" = 7.0, int(1, 16);
        brightness: "shape/brightness" = 0.5, UNIT;
        length: "shape/length" = 1.0, exp(0.5, 2.0);
        speed: "shape/speed" = 1.0, exp(0.5, 2.0);
        click: "shape/click" = 0.5, UNIT;
        variation: "shape/variation" = 0.5, UNIT;
        space: "space/amount" = 0.5, UNIT;
        tail: "space/tail" = 0.5, UNIT;
        width: "space/width" = 0.5, UNIT;
        gain: "master/gain" = 1.0, GAIN;
    }
}

// ---------------------------------------------------------------------------------------------
// Pitch: key, scale, chord tones
// ---------------------------------------------------------------------------------------------

fn style_index(p: &UiParams) -> usize {
    (p.style.round().max(0.0) as usize).min(STYLE_NAMES.len() - 1)
}

fn scale_index(p: &UiParams) -> usize {
    (p.scale.round().max(0.0) as usize).min(SCALES.len() - 1)
}

/// Scale step of chord tone `ct` of the tonic triad: 0 root, 1 third, 2 fifth, 3 the root an
/// octave up, and so on (negative tones go down).
pub fn chord_tone(scale: usize, ct: i32) -> i32 {
    let n = SCALES[scale].len() as i32;
    n * ct.div_euclid(3) + TRIADS[scale][ct.rem_euclid(3) as usize]
}

/// Semitones above the tonic of scale step `step` (negative steps go below it).
pub fn scale_semitones(scale: usize, step: i32) -> i32 {
    let s = SCALES[scale];
    let n = s.len() as i32;
    12 * step.div_euclid(n) + s[step.rem_euclid(n) as usize]
}

/// MIDI note of the kit's tonic: the key's note between G4 and F#5, the register of UI tones.
pub fn tonic_midi(key: usize) -> f32 {
    let key = key.min(11) as f32;
    72.0 + if key >= 7.0 { key - 12.0 } else { key }
}

/// Frequency of scale step `step`, `octave` octaves above the tonic, with the key, scale,
/// register (`music/octave`) and tuning of `p`. The pitch every note of the kit is built from.
pub fn note_hz(p: &UiParams, octave: i32, step: i32) -> f32 {
    let key = p.key.round().max(0.0) as usize;
    let semis = tonic_midi(key) + 12.0 * (octave as f32 + p.octave.round()) + scale_semitones(scale_index(p), step) as f32;
    midi_to_hz(semis + p.tuning / 100.0)
}

/// Scale steps a `combo` input raises an event by: round(combo * 16), capped at
/// `music/combo_max_steps`.
pub fn combo_steps(combo: f32, p: &UiParams) -> i32 {
    ((combo.clamp(0.0, 1.0) * STEPS_PER_UNIT).round() as i32).min(p.combo_max.round() as i32)
}

// ---------------------------------------------------------------------------------------------
// Styles: what one note sounds like
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Wave {
    /// Two-operator FM sine (index follows the envelope).
    Fm,
    /// Band-limited pulse with this duty cycle.
    Pulse(f32),
}

#[derive(Clone, Copy, Debug)]
struct Style {
    wave: Wave,
    fm_ratio: f32,
    fm_index: f32,
    /// The FM index decays this many times faster than the note.
    fm_fade: f32,
    /// Overtones: ratios to the fundamental and levels; they decay `partial_fade` times faster.
    partial: [(f32, f32); 2],
    partial_fade: f32,
    /// A quieter second oscillator this many cents above the note (chorus or shimmer), and its
    /// share. The main voice stays on the note: the louder voice sets the pitch you hear, so a
    /// detune split around the note would leave it flat.
    twin_cents: f32,
    twin_mix: f32,
    attack: f32,
    /// Seconds held at full level per unit of note length (a gate, for the 8-bit style), and the
    /// shortest gate (a sound chip holds a note for at least one 60 Hz frame).
    hold: f32,
    min_hold: f32,
    /// Seconds to fall 60 dB for a note of length 1 at C5.
    decay: f32,
    /// Higher notes decay faster: decay scales by (C5 / f)^tilt.
    tilt: f32,
    /// Pitch glides into each note from this many semitones away, with this time constant.
    glide: f32,
    glide_time: f32,
    /// The strike: band-passed noise at `click_hz` (or `click_rel` times the note when > 0).
    click: f32,
    click_hz: f32,
    click_rel: f32,
    click_q: f32,
    click_decay: f32,
    /// Level of the noise sweep in `ui_open` / `ui_close`.
    swish: f32,
    reverb: f32,
    reverb_time: f32,
    width: f32,
    /// Spacing of arpeggios (8-bit is snappier).
    tempo: f32,
    /// Output trim so the styles match in loudness.
    level: f32,
}

const STYLES: [Style; 5] = [
    // Soft / modern: a round sine with a touch of FM warmth and a soft tick, like phone UIs.
    Style {
        wave: Wave::Fm,
        fm_ratio: 1.0,
        fm_index: 0.9,
        fm_fade: 3.0,
        partial: [(2.0, 0.12), (3.0, 0.0)],
        partial_fade: 2.0,
        twin_cents: 0.0,
        twin_mix: 0.0,
        attack: 0.002,
        hold: 0.0,
        min_hold: 0.0,
        decay: 0.45,
        tilt: 0.4,
        glide: 0.0,
        glide_time: 0.01,
        click: 0.3,
        click_hz: 3000.0,
        click_rel: 0.0,
        click_q: 0.3,
        click_decay: 0.006,
        swish: 0.35,
        reverb: 0.1,
        reverb_time: 0.7,
        width: 0.3,
        tempo: 1.0,
        level: 0.982,
    },
    // Wooden: a marimba bar, fundamental plus its tuned 4th and 10th partials, and a mallet thump.
    Style {
        wave: Wave::Fm,
        fm_ratio: 1.0,
        fm_index: 0.0,
        fm_fade: 1.0,
        partial: [(3.93, 0.32), (9.2, 0.07)],
        partial_fade: 4.0,
        twin_cents: 0.0,
        twin_mix: 0.0,
        attack: 0.0008,
        hold: 0.0,
        min_hold: 0.0,
        decay: 0.55,
        tilt: 0.9,
        glide: 0.0,
        glide_time: 0.01,
        click: 0.4,
        click_hz: 0.0,
        click_rel: 2.2,
        click_q: 0.35,
        click_decay: 0.004,
        swish: 0.25,
        reverb: 0.08,
        reverb_time: 0.5,
        width: 0.35,
        tempo: 1.0,
        level: 0.982,
    },
    // Glass / crystal: an FM ping with inharmonic sparkle, a slow shimmer and a long ring.
    Style {
        wave: Wave::Fm,
        fm_ratio: 3.5,
        fm_index: 0.6,
        fm_fade: 2.5,
        partial: [(2.76, 0.2), (5.4, 0.06)],
        partial_fade: 1.6,
        twin_cents: 3.0,
        twin_mix: 0.35,
        attack: 0.0005,
        hold: 0.0,
        min_hold: 0.0,
        decay: 0.8,
        tilt: 0.2,
        glide: 0.0,
        glide_time: 0.01,
        click: 0.3,
        click_hz: 7000.0,
        click_rel: 0.0,
        click_q: 0.5,
        click_decay: 0.0025,
        swish: 0.3,
        reverb: 0.16,
        reverb_time: 1.0,
        width: 0.6,
        tempo: 1.0,
        level: 0.708,
    },
    // Retro 8-bit: a 25 % pulse, gated like a sound chip channel, almost dry.
    Style {
        wave: Wave::Pulse(0.25),
        fm_ratio: 1.0,
        fm_index: 0.0,
        fm_fade: 1.0,
        partial: [(2.0, 0.0), (3.0, 0.0)],
        partial_fade: 1.0,
        twin_cents: 0.0,
        twin_mix: 0.0,
        attack: 0.0005,
        hold: 0.07,
        min_hold: 0.02,
        decay: 0.07,
        tilt: 0.0,
        glide: 0.0,
        glide_time: 0.01,
        click: 0.08,
        click_hz: 6000.0,
        click_rel: 0.0,
        click_q: 0.2,
        click_decay: 0.003,
        swish: 0.2,
        reverb: 0.05,
        reverb_time: 0.3,
        width: 0.2,
        tempo: 0.85,
        level: 0.435,
    },
    // Sci-fi: a detuned FM voice that chirps up into each note, with more room.
    Style {
        wave: Wave::Fm,
        fm_ratio: 0.5,
        fm_index: 1.3,
        fm_fade: 1.5,
        partial: [(2.0, 0.15), (3.0, 0.0)],
        partial_fade: 1.5,
        twin_cents: 10.0,
        twin_mix: 0.3,
        attack: 0.003,
        hold: 0.0,
        min_hold: 0.0,
        decay: 0.5,
        tilt: 0.3,
        glide: -5.0,
        glide_time: 0.004,
        click: 0.18,
        click_hz: 5000.0,
        click_rel: 0.0,
        click_q: 0.6,
        click_decay: 0.008,
        swish: 0.45,
        reverb: 0.14,
        reverb_time: 0.9,
        width: 0.7,
        tempo: 1.0,
        level: 0.964,
    },
];

// ---------------------------------------------------------------------------------------------
// Events: what is played
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Plain,
    /// The notes rise round(`value` * `music/slider_steps`) scale steps.
    Slider,
    /// Like a slider: `value` picks the step, so a game can give each letter or speaker a pitch.
    Typing,
    /// `count` coins (n / 16) scatter into an arpeggio; one coin is the classic two-note coin.
    Coins,
}

/// One note of an event: `t` seconds after the trigger, `oct` octaves above the tonic, chord tone
/// `ct` plus `st` scale steps. `dur` scales the style's decay, `hold` adds seconds at full level.
#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub t: f32,
    pub oct: i32,
    pub ct: i32,
    pub st: i32,
    pub vel: f32,
    pub dur: f32,
    pub hold: f32,
    pub pan: f32,
}

const N: Note = Note { t: 0.0, oct: 0, ct: 0, st: 0, vel: 0.7, dur: 1.0, hold: 0.0, pan: 0.0 };

/// A band-passed noise sweep from `from` to `to` Hz over `secs` (panels opening and closing).
#[derive(Clone, Copy, Debug)]
pub struct Swish {
    pub from: f32,
    pub to: f32,
    pub secs: f32,
    pub level: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub notes: &'static [Note],
    pub kind: Kind,
    /// Strike level and length relative to the style's.
    pub click: f32,
    pub click_len: f32,
    /// Minimum attack in seconds, 0 = the style's (hover fades in).
    pub attack: f32,
    /// Brightness relative to the style's (error is darker).
    pub bright: f32,
    pub swish: Option<Swish>,
    /// Loudness trim of the event within the kit.
    pub gain: f32,
}

const EV: Event = Event { notes: &[], kind: Kind::Plain, click: 1.0, click_len: 1.0, attack: 0.0, bright: 1.0, swish: None, gain: 1.0 };

/// A UI event: a name and a score. [`Ui`] turns it into a generator.
pub trait UiEvent: Send + 'static {
    const NAME: &'static str;
    const DOC: &'static str;
    const INPUTS: &'static [InputSpec];
    const EVENT: Event;
}

const POWER: InputSpec = InputSpec { name: "power", default: 1.0, doc: "Light touch to a firm press: level and brightness" };
const DISTANCE: InputSpec = InputSpec { name: "distance", default: 0.0, doc: "0 = foreground UI, 1 = far off or behind a dialog (duller, quieter, roomier)" };
const COMBO: InputSpec = InputSpec { name: "combo", default: 0.0, doc: "Streak or chain position as n / 16: each 1/16 raises the sound one step of the scale" };
const VALUE: InputSpec = InputSpec { name: "value", default: 0.5, doc: "Slider position 0..1: the tick's pitch follows it up the scale" };
const LETTER: InputSpec = InputSpec { name: "value", default: 0.0, doc: "Pitch of this blip 0..1, snapped to the scale: randf() per letter babbles, a constant gives a speaker a voice" };
const COUNT: InputSpec = InputSpec { name: "count", default: 0.0, doc: "Coins in the burst as n / 16 (0 or 1/16 = one coin, 1 = sixteen)" };
const BASIC: &[InputSpec] = &[POWER, DISTANCE, COMBO];

macro_rules! ui_event {
    ($ty:ident, $name:literal, $inputs:expr, $doc:literal, $event:expr) => {
        pub struct $ty;
        impl UiEvent for $ty {
            const NAME: &'static str = $name;
            const DOC: &'static str = $doc;
            const INPUTS: &'static [InputSpec] = $inputs;
            const EVENT: Event = $event;
        }
    };
}

ui_event!(Hover, "ui_hover", BASIC, "Pointer over a button: a soft, high, fading-in blip, the quietest of the kit.",
    Event { notes: &[Note { oct: 1, ct: 2, vel: 0.5, dur: 0.3, ..N }], click: 0.35, click_len: 2.0, attack: 0.015, bright: 0.8, gain: 0.508, ..EV });
ui_event!(Click, "ui_click", BASIC, "Click or tap: a short tick and tone on the tonic.",
    Event { notes: &[Note { oct: 1, vel: 0.8, dur: 0.16, ..N }], click: 1.2, click_len: 3.5, gain: 0.507, ..EV });
ui_event!(ToggleOn, "ui_toggle_on", BASIC, "Switch turned on: two quick notes rising root to fifth.",
    Event { notes: &[Note { oct: 1, vel: 0.6, dur: 0.3, ..N }, Note { t: 0.05, oct: 1, ct: 2, vel: 0.75, dur: 0.45, ..N }], click: 0.8, click_len: 2.0, gain: 0.519, ..EV });
ui_event!(ToggleOff, "ui_toggle_off", BASIC, "Switch turned off: the same two notes falling, a little darker.",
    Event { notes: &[Note { oct: 1, ct: 2, vel: 0.65, dur: 0.3, ..N }, Note { t: 0.05, oct: 1, vel: 0.6, dur: 0.45, ..N }], click: 0.8, click_len: 2.0, bright: 0.85, gain: 0.603, ..EV });
ui_event!(Confirm, "ui_confirm", BASIC, "Confirm or OK: the fifth resolving up to the tonic.",
    Event { notes: &[Note { ct: 2, vel: 0.7, dur: 0.5, ..N }, Note { t: 0.075, oct: 1, vel: 0.9, dur: 1.0, ..N }], click: 0.6, gain: 0.569, ..EV });
ui_event!(Cancel, "ui_cancel", BASIC, "Cancel or back: the third falling to the tonic, softer than confirm.",
    Event { notes: &[Note { ct: 1, vel: 0.7, dur: 0.45, ..N }, Note { t: 0.065, vel: 0.65, dur: 0.7, ..N }], click: 0.6, bright: 0.85, gain: 0.751, ..EV });
ui_event!(Error, "ui_error", BASIC, "Error or denied: two low, dark notes falling a scale step (a semitone in major), an octave under the kit.",
    Event {
        notes: &[Note { oct: -1, vel: 0.85, dur: 0.25, ..N }, Note { t: 0.11, oct: -1, st: -1, vel: 0.8, dur: 0.5, ..N }],
        click: 0.5,
        bright: 0.5,
        gain: 0.575, ..EV
    });
ui_event!(Notify, "ui_notify", BASIC, "Notification: a bright two-note chime that rings a little longer.",
    Event { notes: &[Note { ct: 2, vel: 0.7, dur: 1.0, pan: -0.3, ..N }, Note { t: 0.11, oct: 1, ct: 1, vel: 0.8, dur: 1.5, pan: 0.3, ..N }], click: 0.4, gain: 0.562, ..EV });
ui_event!(Open, "ui_open", BASIC, "Panel or menu opening: a rising swish under two soft notes.",
    Event {
        notes: &[Note { vel: 0.45, dur: 0.45, ..N }, Note { t: 0.06, ct: 2, vel: 0.55, dur: 0.7, ..N }],
        click: 0.3,
        swish: Some(Swish { from: 500.0, to: 4000.0, secs: 0.16, level: 0.5 }),
        gain: 0.631, ..EV
    });
ui_event!(Close, "ui_close", BASIC, "Panel or menu closing: a falling swish under the notes reversed.",
    Event {
        notes: &[Note { ct: 2, vel: 0.5, dur: 0.45, ..N }, Note { t: 0.06, vel: 0.5, dur: 0.7, ..N }],
        click: 0.3,
        swish: Some(Swish { from: 4000.0, to: 500.0, secs: 0.16, level: 0.5 }),
        gain: 0.562, ..EV
    });
ui_event!(Slider, "ui_slider", &[POWER, DISTANCE, COMBO, VALUE], "Slider detent: a short tick whose pitch follows `value` up the scale (`music/slider_steps` steps end to end).",
    Event { notes: &[Note { vel: 0.55, dur: 0.15, ..N }], kind: Kind::Slider, click: 0.9, click_len: 2.5, gain: 0.747, ..EV });
ui_event!(Typing, "ui_type", &[POWER, DISTANCE, COMBO, LETTER], "Text blip, one per letter: very short, on the scale step `value` picks (up to `music/slider_steps`).",
    Event { notes: &[Note { oct: 1, vel: 0.45, dur: 0.14, ..N }], kind: Kind::Typing, click: 1.1, click_len: 3.0, gain: 0.962, ..EV });
ui_event!(Coin, "ui_coin", &[POWER, DISTANCE, COMBO, COUNT], "Coin: the two-note coin; with `count` a burst of coins scattering into a quick arpeggio.",
    Event { notes: &[Note { ct: 2, vel: 0.7, dur: 0.3, ..N }, Note { t: 0.07, oct: 1, vel: 0.9, dur: 1.1, ..N }], kind: Kind::Coins, click: 0.6, gain: 0.542, ..EV });
ui_event!(Collect, "ui_collect", BASIC, "Collect or pick up: a fast triad run up to the fifth.",
    Event {
        notes: &[Note { oct: 1, vel: 0.6, dur: 0.35, pan: -0.3, ..N }, Note { t: 0.045, oct: 1, ct: 1, vel: 0.7, dur: 0.35, ..N }, Note { t: 0.09, oct: 1, ct: 2, vel: 0.85, dur: 0.9, pan: 0.3, ..N }],
        click: 0.5,
        gain: 0.524,
        ..EV
    });
ui_event!(Combo, "ui_combo", BASIC, "Match or combo hit: a plucked note with its octave; raise `combo` per match and a chain climbs the scale.",
    Event { notes: &[Note { vel: 0.85, dur: 0.8, ..N }, Note { oct: 1, vel: 0.3, dur: 0.5, ..N }], click: 0.8, gain: 0.427, ..EV });
ui_event!(LevelUp, "ui_level_up", BASIC, "Level up or achievement: a two-octave arpeggio landing on a ringing chord with a sparkle on top, about 1.5 s.",
    Event {
        notes: &[
            Note { vel: 0.55, dur: 0.5, pan: -0.5, ..N },
            Note { t: 0.06, ct: 1, vel: 0.6, dur: 0.5, pan: -0.3, ..N },
            Note { t: 0.12, ct: 2, vel: 0.62, dur: 0.5, pan: -0.1, ..N },
            Note { t: 0.18, ct: 3, vel: 0.65, dur: 0.5, pan: 0.1, ..N },
            Note { t: 0.24, ct: 4, vel: 0.68, dur: 0.5, pan: 0.3, ..N },
            Note { t: 0.30, ct: 5, vel: 0.7, dur: 0.6, pan: 0.5, ..N },
            Note { t: 0.38, ct: 3, vel: 0.55, dur: 2.2, pan: -0.4, ..N },
            Note { t: 0.38, ct: 4, vel: 0.5, dur: 2.2, ..N },
            Note { t: 0.38, ct: 5, vel: 0.5, dur: 2.2, pan: 0.4, ..N },
            Note { t: 0.40, ct: 6, vel: 0.4, dur: 1.8, ..N },
        ],
        click: 0.5,
        gain: 0.462,
        ..EV
    });
ui_event!(Countdown, "ui_countdown", BASIC, "Countdown tick (3, 2, 1): a short held beep on the tonic.",
    Event { notes: &[Note { vel: 0.8, dur: 0.45, hold: 0.05, ..N }], click: 0.4, gain: 0.371, ..EV });
ui_event!(Go, "ui_go", BASIC, "Countdown go: the tonic an octave up over its fifth, held longer.",
    Event { notes: &[Note { oct: 1, vel: 0.85, dur: 1.0, hold: 0.15, ..N }, Note { ct: 2, vel: 0.45, dur: 1.0, hold: 0.15, ..N }], click: 0.5, gain: 0.342, ..EV });
ui_event!(Fanfare, "ui_fanfare", BASIC, "Reward fanfare: three pickup notes into a full chord over the tonic below, about 2 s.",
    Event {
        notes: &[
            Note { ct: 2, vel: 0.6, dur: 0.35, ..N },
            Note { t: 0.11, ct: 2, vel: 0.6, dur: 0.35, ..N },
            Note { t: 0.22, ct: 2, vel: 0.65, dur: 0.35, ..N },
            Note { t: 0.34, oct: -1, vel: 0.5, dur: 2.4, ..N },
            Note { t: 0.34, ct: 2, vel: 0.55, dur: 2.4, pan: -0.4, ..N },
            Note { t: 0.34, ct: 3, vel: 0.7, dur: 2.4, ..N },
            Note { t: 0.34, ct: 4, vel: 0.55, dur: 2.4, pan: 0.4, ..N },
        ],
        click: 0.5,
        gain: 0.339,
        ..EV
    });

// ---------------------------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------------------------

/// Fold a note down by octaves until it is at most MAX_HZ.
fn fold(mut hz: f32) -> f32 {
    while hz > MAX_HZ {
        hz *= 0.5;
    }
    hz
}

/// sin(2 pi x) for any x, to about 1e-3: a parabola with one refinement step.
#[inline]
fn sin01(x: f32) -> f32 {
    let x = x - (x + 0.5).floor();
    let y = 8.0 * x - 16.0 * x * x.abs();
    0.225 * (y * y.abs() - y) + y
}

/// Fixed, different start phases for the oscillators of the `i`th note (carrier, modulator,
/// twin, partials), so chord tones and partials do not all peak together at their onset (the
/// attack ramp starts at zero either way). The twin starts with the carrier: half a cycle apart
/// they would cancel for most of a beat. The modulator starts at `ratio` times the carrier's
/// phase, which keeps the FM waveform odd-symmetric: at ratios 1 and 0.5 a sideband lands on
/// 0 Hz, and any other alignment turns it into a DC offset.
fn start_phases(i: usize, fm_ratio: f32) -> [f32; 6] {
    let g = (0.618_034 * i as f32).fract();
    let m = (g * fm_ratio).fract();
    [g, m, g, (g + 0.33).fract(), (g + 0.71).fract(), m]
}

/// Balance-law pan: centre is (1, 1), so a width of zero leaves the mono signal untouched.
#[inline]
fn balance(pan: f32) -> (f32, f32) {
    ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
}

#[derive(Clone, Copy)]
struct Voice {
    on: bool,
    style: u8,
    /// Seconds since the note's start (negative while it waits its turn).
    t: f32,
    hz: f32,
    vel: f32,
    pan: f32,
    attack: f32,
    hold: f32,
    amp: f32,
    amp_k: f32,
    index: f32,
    index_k: f32,
    part: f32,
    part_k: f32,
    click: f32,
    click_k: f32,
    twin_cents: f32,
    twin_mix: f32,
    /// Carrier, modulator, twin, two partials, the twin's modulator.
    phase: [f32; 6],
    svf: Svf,
}

impl Voice {
    fn idle() -> Self {
        Voice {
            on: false,
            style: 0,
            t: 0.0,
            hz: 440.0,
            vel: 0.0,
            pan: 0.0,
            attack: 0.001,
            hold: 0.0,
            amp: 0.0,
            amp_k: 0.0,
            index: 0.0,
            index_k: 0.0,
            part: 0.0,
            part_k: 0.0,
            click: 0.0,
            click_k: 0.0,
            twin_cents: 0.0,
            twin_mix: 0.0,
            phase: [0.0; 6],
            svf: Svf::default(),
        }
    }
}

#[derive(Clone, Copy)]
struct SwishState {
    on: bool,
    t: f32,
    sweep: Swish,
    svf: Svf,
}

/// Renders any [`Event`] in any style; one per generator instance.
pub struct UiKit {
    sr: f32,
    seed: u32,
    voices: [Voice; MAX_VOICES],
    osc: [[Oscillator; 2]; MAX_VOICES],
    swish: SwishState,
    noise: Noise,
    rng: Rng,
    reverb: Reverb,
    far: [[OnePole; 2]; 2],
    tail_left: f32,
    /// Style, power and distance of the latest trigger (the bus follows them).
    style: usize,
    power: f32,
    distance: f32,
    pitch_ratio: f32,
}

impl UiKit {
    pub fn new(sr: f32, seed: u32) -> Self {
        UiKit {
            sr,
            seed,
            voices: [Voice::idle(); MAX_VOICES],
            osc: core::array::from_fn(|k| [Oscillator::new(mix_seed(seed + 2 * k as u32)), Oscillator::new(mix_seed(seed + 2 * k as u32 + 1))]),
            swish: SwishState { on: false, t: 0.0, sweep: Swish { from: 1000.0, to: 1000.0, secs: 0.1, level: 0.0 }, svf: Svf::default() },
            noise: Noise::new(mix_seed(seed ^ 0x5F5F)),
            rng: Rng::new(mix_seed(seed ^ 0xA1A1)),
            reverb: Reverb::new(sr),
            far: [[OnePole::default(); 2]; 2],
            tail_left: 0.0,
            style: 0,
            power: 1.0,
            distance: 0.0,
            pitch_ratio: 1.0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.tail_left > 0.0 || self.swish.on || self.voices.iter().any(|v| v.on)
    }

    pub fn set_pitch_ratio(&mut self, ratio: f32) {
        self.pitch_ratio = ratio;
    }

    fn send(style: &Style, p: &UiParams, distance: f32) -> f32 {
        (style.reverb * 2.0 * p.space * (1.0 + 2.0 * distance)).min(1.2)
    }

    fn reverb_time(style: &Style, p: &UiParams) -> f32 {
        style.reverb_time * (0.4 + 1.2 * p.tail)
    }

    /// The notes of one trigger, and the scale steps `value` raises them; returns how many.
    fn score(ev: &Event, p: &UiParams, extra: f32, rng: &mut Rng, out: &mut [Note; MAX_NOTES]) -> (usize, i32) {
        let v = p.variation;
        match ev.kind {
            Kind::Coins if (extra * STEPS_PER_UNIT).round() >= 2.0 => {
                let n = ((extra * STEPS_PER_UNIT).round() as usize).min(MAX_COINS);
                for (k, note) in out.iter_mut().enumerate().take(n) {
                    let last = k + 1 == n;
                    // Up the triad from the coin's top note, two octaves, then round again:
                    // coins tumbling.
                    let side = if k % 2 == 0 { -0.5 } else { 0.5 };
                    *note = Note {
                        t: k as f32 * COIN_GAP,
                        ct: 3 + (k % 6) as i32,
                        vel: if last { 0.85 } else { 0.62 },
                        dur: if last { 1.0 } else { 0.5 },
                        pan: side * (1.0 - v + v * rng.next_f32()),
                        ..N
                    };
                }
                (n, 0)
            }
            _ => {
                let n = ev.notes.len().min(MAX_NOTES);
                out[..n].copy_from_slice(&ev.notes[..n]);
                let shift = match ev.kind {
                    Kind::Slider | Kind::Typing => (extra.clamp(0.0, 1.0) * p.slider_steps.round()).round() as i32,
                    _ => 0,
                };
                (n, shift)
            }
        }
    }

    pub fn trigger(&mut self, ev: &Event, p: &UiParams, x: &[f32]) {
        let input = |i: usize, d: f32| x.get(i).copied().unwrap_or(d);
        let (power, distance, combo, extra) = (input(0, 1.0), input(1, 0.0), input(2, 0.0), input(3, 0.0));
        if p.variation <= 0.0 {
            // No variation means none at all: the same trigger renders the same samples.
            self.noise = Noise::new(mix_seed(self.seed ^ 0x5F5F));
            self.rng = Rng::new(mix_seed(self.seed ^ 0xA1A1));
            if !self.voices.iter().any(|v| v.on) {
                self.reverb.clear();
                self.far = [[OnePole::default(); 2]; 2];
            }
        }
        self.power = power;
        self.distance = distance;
        self.style = style_index(p);
        let st = STYLES[self.style];
        let v = p.variation;
        // Per trigger: a few cents of detune for the whole event (intervals stay pure), level.
        let detune = (v * 4.0 * self.rng.next_bipolar() / 1200.0).exp2();
        let level = 1.0 + v * 0.2 * self.rng.next_bipolar();
        let mut notes = [N; MAX_NOTES];
        let (count, shift) = Self::score(ev, p, extra, &mut self.rng, &mut notes);
        let steps = combo_steps(combo, p) + shift;
        let scale = scale_index(p);
        let bright = (2.0 * (p.brightness - 0.5)).exp2() * ev.bright * (0.7 + 0.3 * power);
        let tempo = st.tempo / p.speed;
        for (i, n) in notes[..count].iter().enumerate() {
            let step = chord_tone(scale, n.ct) + n.st;
            let hz = fold(note_hz(p, n.oct, step + steps) * detune);
            // Decay follows the written note: a combo chain or a slider rings the same at any height.
            let written = fold(note_hz(p, n.oct, step) * detune).min(0.5 * MAX_HZ);
            let jitter = if i == 0 { 0.0 } else { 0.006 * v * self.rng.next_f32() };
            let t60 = (st.decay * n.dur * p.length * (REF_HZ / written).powf(st.tilt)).max(0.01);
            let amp_k = (-6.9 / (t60 * self.sr)).exp();
            let click_hz = if st.click_rel > 0.0 { hz * st.click_rel } else { st.click_hz } * bright.sqrt() * (1.0 + 0.25 * v * self.rng.next_bipolar());
            let k = self.free_voice();
            let mut svf = Svf::default();
            svf.set(FilterMode::BandPass, click_hz.min(self.sr * 0.45), st.click_q, self.sr);
            self.voices[k] = Voice {
                on: true,
                style: self.style as u8,
                t: -(n.t * tempo + jitter),
                hz,
                vel: n.vel * ev.gain * level * (0.3 + 0.7 * power),
                pan: (n.pan + 0.15 * v * self.rng.next_bipolar()).clamp(-1.0, 1.0),
                attack: st.attack.max(ev.attack).max(0.0005),
                hold: ((st.hold * n.dur).max(st.min_hold) + n.hold) * p.length,
                amp: 1.0,
                amp_k,
                index: st.fm_index * bright,
                index_k: amp_k.powf(st.fm_fade),
                part: bright.sqrt(),
                part_k: amp_k.powf(st.partial_fade - 1.0),
                click: ev.click * st.click * 2.0 * p.click * (0.4 + 0.6 * power) * (1.0 + 0.2 * v * self.rng.next_bipolar()),
                click_k: (-6.9 / (st.click_decay * ev.click_len * self.sr)).exp(),
                twin_cents: st.twin_cents,
                twin_mix: st.twin_mix,
                phase: start_phases(i, st.fm_ratio),
                svf,
            };
            let ph = start_phases(i, st.fm_ratio);
            self.osc[k][0].reset(ph[0]);
            self.osc[k][1].reset(ph[2]);
        }
        if let Some(sweep) = ev.swish {
            let mut svf = Svf::default();
            svf.set(FilterMode::BandPass, sweep.from, 0.5, self.sr);
            self.swish = SwishState { on: true, t: -0.0, sweep: Swish { secs: sweep.secs * tempo, level: sweep.level * st.swish * (0.3 + 0.7 * power) * ev.gain, ..sweep }, svf };
        }
    }

    /// An idle voice, else the quietest one.
    fn free_voice(&self) -> usize {
        if let Some(k) = self.voices.iter().position(|v| !v.on) {
            return k;
        }
        let loudness = |v: &Voice| if v.t < 0.0 { f32::MAX } else { v.amp * v.vel };
        (0..MAX_VOICES).min_by(|&a, &b| loudness(&self.voices[a]).total_cmp(&loudness(&self.voices[b]))).unwrap_or(0)
    }

    /// Upper bound on one trigger of `ev` with params `p` (any inputs), tail included.
    pub fn length(ev: &Event, p: &UiParams) -> f32 {
        let st = &STYLES[style_index(p)];
        let tempo = st.tempo / p.speed;
        let jitter = 0.006 * p.variation;
        // Decay follows the written note (see `trigger`), detuned flat as far as variation goes.
        let scale = scale_index(p);
        let end = |n: &Note| {
            let f = fold(note_hz(p, n.oct, chord_tone(scale, n.ct) + n.st) * (-4.0 * p.variation / 1200.0).exp2()).min(0.5 * MAX_HZ);
            let t60 = (st.decay * n.dur * p.length * (REF_HZ / f).powf(st.tilt)).max(0.01);
            n.t * tempo + jitter + st.attack.max(ev.attack).max(0.0005) + ((st.hold * n.dur).max(st.min_hold) + n.hold) * p.length + t60
        };
        let mut last = ev.notes.iter().map(end).fold(0.0, f32::max);
        if ev.kind == Kind::Coins {
            let n = Note { t: (MAX_COINS - 1) as f32 * COIN_GAP, ct: 3, ..N };
            last = last.max(end(&n));
        }
        if let Some(s) = ev.swish {
            last = last.max(s.secs * tempo);
        }
        let tail = if Self::send(st, p, 0.0) > 0.0 { Self::reverb_time(st, p) } else { 0.0 };
        last + tail + 2.0 * BLOCK as f32 / 8000.0 + SETTLE
    }

    /// Render one block (`left.len() <= BLOCK`), overwriting both channels. `width` 0 renders
    /// mono into `left` and copies it.
    pub fn render(&mut self, p: &UiParams, width: f32, left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        let sr = self.sr;
        let dt = 1.0 / sr;
        let stereo = width > 0.0;
        left.iter_mut().chain(right.iter_mut()).for_each(|s| *s = 0.0);
        let UiKit { voices, osc, noise, swish, .. } = self;
        let mut any = false;
        for (v, osc) in voices.iter_mut().zip(osc.iter_mut()) {
            if !v.on {
                continue;
            }
            any = true;
            let st = &STYLES[v.style as usize];
            let glide = if st.glide != 0.0 { (st.glide * (-v.t.max(0.0) / st.glide_time).exp() / 12.0).exp2() } else { 1.0 };
            let inc = (v.hz * glide * self.pitch_ratio / sr).min(0.45);
            let (inc_a, inc_b) = (inc, inc * (v.twin_cents / 1200.0).exp2());
            let (p1, p2) = (st.partial[0], st.partial[1]);
            let (gl, gr) = if stereo { balance(v.pan * width) } else { (1.0, 1.0) };
            let attack_end = v.attack + v.hold;
            for i in 0..n {
                v.t += dt;
                if v.t < 0.0 {
                    continue;
                }
                let env = if v.t < v.attack {
                    v.t / v.attack
                } else if v.t < attack_end {
                    1.0
                } else {
                    v.amp *= v.amp_k;
                    v.index *= v.index_k;
                    v.part *= v.part_k;
                    (v.amp - EPS) * (1.0 / (1.0 - EPS))
                };
                if env <= 0.0 {
                    v.on = false;
                    break;
                }
                let mut y = match st.wave {
                    Wave::Fm => {
                        let ph = &mut v.phase;
                        let depth = v.index * (1.0 / core::f32::consts::TAU);
                        ph[0] += inc_a;
                        ph[1] += inc_a * st.fm_ratio;
                        let mut y = sin01(ph[0] + depth * sin01(ph[1]));
                        if v.twin_mix > 0.0 {
                            // The twin has its own modulator: sharing one would leave a
                            // sideband a few Hz from zero, a sub-audio wobble.
                            ph[2] += inc_b;
                            ph[5] += inc_b * st.fm_ratio;
                            y += v.twin_mix * (sin01(ph[2] + depth * sin01(ph[5])) - y);
                        }
                        y
                    }
                    Wave::Pulse(pw) => {
                        let mut y = osc[0].next(Waveform::Pulse, inc_a, pw);
                        if v.twin_mix > 0.0 {
                            y += v.twin_mix * (osc[1].next(Waveform::Pulse, inc_b, pw) - y);
                        }
                        y
                    }
                };
                if p1.1 > 0.0 {
                    let ph = &mut v.phase;
                    ph[3] += inc * p1.0;
                    ph[4] += inc * p2.0;
                    y += v.part * (p1.1 * sin01(ph[3]) + p2.1 * sin01(ph[4]));
                }
                let mut s = y * env * v.vel;
                if v.click > 0.0 {
                    let ramp = (v.t * 3000.0).min(1.0);
                    s += v.svf.tick(noise.white()) * (CLICK_GAIN * v.click * ramp * v.vel);
                    v.click *= v.click_k;
                    if v.click < 1e-5 {
                        v.click = 0.0;
                    }
                }
                left[i] += s * gl;
                if stereo {
                    right[i] += s * gr;
                }
            }
            // Keep the phases small; each only matters modulo 1.
            for ph in v.phase.iter_mut() {
                *ph -= ph.floor();
            }
        }
        if swish.on {
            any = true;
            let s = swish.sweep;
            let frac = (swish.t / s.secs).clamp(0.0, 1.0);
            swish.svf.set(FilterMode::BandPass, s.from * (s.to / s.from).powf(frac), 0.5, sr);
            for i in 0..n {
                swish.t += dt;
                if swish.t >= s.secs {
                    swish.on = false;
                    break;
                }
                // sin^2 window: starts and ends at exactly zero.
                let w = sin01(0.5 * swish.t / s.secs);
                let y = swish.svf.tick(noise.white()) * w * w * s.level * 2.0;
                left[i] += y;
                if stereo {
                    right[i] += y;
                }
            }
        }
        let st = STYLES[self.style];
        let rt = Self::reverb_time(&st, p);
        let send = Self::send(&st, p, self.distance);
        let tail_start = self.tail_left;
        if any {
            self.tail_left = if send > 0.0 { rt } else { 0.0 };
        } else {
            self.tail_left = (self.tail_left - n as f32 * dt).max(0.0);
        }
        // Wet gain ramps across the block over the last TAIL_FADE seconds of the tail.
        let (fade_a, fade_b) = if any { (1.0, 1.0) } else { ((tail_start / TAIL_FADE).min(1.0), (self.tail_left / TAIL_FADE).min(1.0)) };
        let fade_step = (fade_b - fade_a) / n as f32;
        // Distance: duller, quieter, more room than source.
        let far = hz_coef(18000.0 * (400.0f32 / 18000.0).powf(self.distance), sr);
        let dry = (1.0 - 0.75 * self.distance) * p.gain * st.level;
        let damping = 0.35 + 0.4 * self.distance;
        let mut fade = fade_a;
        if !stereo {
            for (ol, or) in left.iter_mut().zip(right.iter_mut()) {
                fade += fade_step;
                let x = self.far[0][0].lp(*ol, far);
                let x = self.far[0][1].lp(x, far);
                *ol = (x + self.reverb.tick(x, rt, damping) * send * fade) * dry;
                *or = *ol;
            }
            return;
        }
        for (ol, or) in left.iter_mut().zip(right.iter_mut()) {
            fade += fade_step;
            let xl = self.far[0][0].lp(*ol, far);
            let xl = self.far[0][1].lp(xl, far);
            let xr = self.far[1][0].lp(*or, far);
            let xr = self.far[1][1].lp(xr, far);
            let (mid, side) = self.reverb.tick_stereo(0.5 * (xl + xr), rt, damping);
            *ol = (xl + (mid + side * width) * send * fade) * dry;
            *or = (xr + (mid - side * width) * send * fade) * dry;
        }
    }
}

/// Adapter: any [`UiEvent`] as a one-shot [`Generator`].
pub struct Ui<E: UiEvent> {
    kit: UiKit,
    scratch: [f32; BLOCK],
    event: PhantomData<E>,
}

fn style_preset(style: usize) -> UiParams {
    UiParams { style: style as f32, ..UiParams::default() }
}

impl<E: UiEvent> Generator for Ui<E> {
    type P = UiParams;
    const NAME: &'static str = E::NAME;
    const CATEGORY: &'static str = "ui";
    const DOC: &'static str = E::DOC;
    const INPUTS: &'static [InputSpec] = E::INPUTS;
    const INPUT_SMOOTH_SECS: f32 = 0.0;
    const ONE_SHOT: bool = true;
    const TRANSPOSES: bool = true;

    /// The five styles, everything else at its default: loading one re-skins the event and
    /// resets the key to C major, so set `music/*` after it (or set `kit/style` alone).
    fn presets() -> Vec<(&'static str, UiParams)> {
        STYLE_NAMES.iter().enumerate().map(|(i, &name)| (name, style_preset(i))).collect()
    }

    fn new(sr: f32) -> Self {
        let seed = E::NAME.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
        Ui { kit: UiKit::new(sr, seed), scratch: [0.0; BLOCK], event: PhantomData }
    }

    fn trigger(&mut self, x: &[f32], p: &UiParams) {
        self.kit.trigger(&E::EVENT, p, x);
    }

    fn is_active(&self) -> bool {
        self.kit.is_active()
    }

    fn length(p: &UiParams) -> Option<f32> {
        Some(UiKit::length(&E::EVENT, p))
    }

    fn set_pitch_ratio(&mut self, ratio: f32) {
        self.kit.set_pitch_ratio(ratio);
    }

    /// The mono render is the event at width zero.
    fn block(&mut self, _x: &[f32], p: &UiParams, out: &mut [f32]) {
        let n = out.len();
        self.kit.render(p, 0.0, out, &mut self.scratch[..n]);
    }

    fn block_stereo(&mut self, _x: &[f32], p: &UiParams, left: &mut [f32], right: &mut [f32]) {
        let width = (STYLES[style_index(p)].width * 2.0 * p.width).clamp(0.0, 1.0);
        self.kit.render(p, width, left, right);
    }
}
