//! Voices: dialogue babble and creature calls, rendered the way a game drives them.
//!   cargo run -p brusverk-core --release --example render_voice -- talk <out.wav> <preset> "<text>" [mood] [anger] [chars/s] [param=v ...]
//!       one line of dialogue, one trigger per character as it types out (spaces and punctuation too)
//!   cargo run -p brusverk-core --release --example render_voice -- type <out.wav> "<text>" [chars/s]
//!       the same line on `ui_type` (one blip per letter, `value` = code / 128), for comparison
//!   cargo run -p brusverk-core --release --example render_voice -- call <out.wav> <generator> <preset> <size> <aggression> <excitement> [times] [gap s]
//!       a creature call, repeated `times` (default 1) every `gap` seconds
//!   cargo run -p brusverk-core --release --example render_voice -- idle <out.wav> <preset> <secs> <excitement> <purr> <size>
//!   cargo run -p brusverk-core --release --example render_voice -- demo <dir>
//!       the demo pieces: dialogue.wav (three characters and moods), creatures.wav (a mouse squeak
//!       up to a big growl), purr.wav (a cat purring), each mono 16-bit
use brusverk_core::generators::{self, voice};
use brusverk_core::Model;

const SR: f32 = 48000.0;

fn write(path: &str, x: &[f32]) {
    let spec = hound::WavSpec { channels: 1, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for s in x {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

fn create(name: &str, preset: &str) -> Box<dyn Model> {
    let mut m = generators::create(name, SR).unwrap_or_else(|| panic!("no generator {name}"));
    if !preset.is_empty() {
        let i = m.desc().preset_index(preset).unwrap_or_else(|| panic!("{name} has no preset {preset}"));
        m.load_preset(i);
    }
    m
}

/// Render `m` into `out` from `t0` for `secs` (mixing in).
fn render_into(m: &mut dyn Model, out: &mut [f32], t0: f32, secs: f32, gain: f32) {
    let a = (t0 * SR) as usize;
    let b = ((t0 + secs) * SR) as usize;
    let mut buf = vec![0.0; b.saturating_sub(a)];
    m.render_mono(&mut buf);
    let len = out.len();
    for (o, s) in out[a.min(len)..b.min(len)].iter_mut().zip(&buf) {
        *o += s * gain;
    }
}

/// Babble `text` into `out` starting at `t0`, `cps` characters a second; returns the end time.
fn talk(m: &mut dyn Model, out: &mut [f32], t0: f32, text: &str, mood: f32, anger: f32, cps: f32) -> f32 {
    m.set_input_by_name("mood", mood);
    m.set_input_by_name("anger", anger);
    let step = 1.0 / cps;
    let mut t = t0;
    for c in text.chars() {
        m.set_input_by_name("letter", voice::letter_value(c));
        m.trigger();
        // Punctuation holds the typing a little, as dialogue boxes do.
        let wait = match c {
            '.' | '!' | '?' => 6.0 * step,
            ',' => 3.0 * step,
            _ => step,
        };
        render_into(m, out, t, wait, 1.0);
        t += wait;
    }
    render_into(m, out, t, 1.5, 1.0);
    t + 0.4
}

fn set_params(m: &mut dyn Model, args: &[String]) {
    for a in args {
        if let Some((k, v)) = a.split_once('=') {
            assert!(m.set_param_by_name(k, v.parse().unwrap()), "no param {k}");
        }
    }
}

fn call(m: &mut dyn Model, out: &mut [f32], t: f32, size: f32, aggr: f32, exc: f32, power: f32) {
    m.set_input_by_name("power", power);
    m.set_input_by_name("size", size);
    m.set_input_by_name("aggression", aggr);
    m.set_input_by_name("excitement", exc);
    m.trigger();
    let secs = m.length_secs().unwrap_or(3.0);
    render_into(m, out, t, secs, 1.0);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let f = |i: usize, d: f32| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    match args.first().map(String::as_str) {
        Some("talk") => {
            let mut m = create("babble", &args[2]);
            set_params(m.as_mut(), args.get(7..).unwrap_or(&[]));
            let mut out = vec![0.0; (SR * 30.0) as usize];
            let end = talk(m.as_mut(), &mut out, 0.1, &args[3], f(4, 0.5), f(5, 0.0), f(6, 14.0));
            out.truncate((end * SR) as usize);
            write(&args[1], &out);
        }
        Some("type") => {
            let mut m = create("ui_type", "");
            m.set_param_by_name("music/slider_steps", 16.0);
            let cps = f(3, 14.0);
            let mut out = vec![0.0; (SR * 30.0) as usize];
            let mut t = 0.1;
            for c in args[2].chars() {
                if c.is_alphanumeric() {
                    m.set_input_by_name("value", voice::letter_value(c));
                    m.trigger();
                }
                render_into(m.as_mut(), &mut out, t, 1.0 / cps, 1.0);
                t += 1.0 / cps;
            }
            render_into(m.as_mut(), &mut out, t, 1.0, 1.0);
            out.truncate(((t + 1.0) * SR) as usize);
            write(&args[1], &out);
        }
        Some("call") => {
            let mut m = create(&args[2], &args[3]);
            let (times, gap) = (f(7, 1.0) as usize, f(8, 2.0));
            let mut out = vec![0.0; (SR * (times as f32 * gap + 8.0)) as usize];
            for k in 0..times {
                call(m.as_mut(), &mut out, 0.05 + k as f32 * gap, f(4, 0.5), f(5, 0.5), f(6, 0.5), 1.0);
            }
            let end = out.iter().rposition(|s| s.abs() > 1e-4).unwrap_or(0) + 4800;
            out.truncate(end.min(out.len()));
            write(&args[1], &out);
        }
        Some("idle") => {
            let mut m = create("creature_idle", &args[2]);
            m.set_input_by_name("excitement", f(4, 0.2));
            m.set_input_by_name("purr", f(5, 0.0));
            m.set_input_by_name("size", f(6, 0.3));
            m.snap();
            let mut out = vec![0.0; (SR * f(3, 8.0)) as usize];
            m.render_mono(&mut out);
            write(&args[1], &out);
        }
        Some("demo") => demo(&args[1]),
        _ => eprintln!("usage: see the header of examples/render_voice.rs"),
    }
}

fn demo(dir: &str) {
    // Dialogue: three characters and moods, about 24 s.
    let mut out = vec![0.0; (SR * 26.0) as usize];
    let mut t = 0.2;
    let lines: [(&str, &str, f32, f32); 4] = [
        ("Critter", "Oh hi! Welcome to our little town. Want some peaches?", 0.9, 0.0),
        ("Elder", "Hmm... the old bridge fell down again, I fear.", 0.15, 0.0),
        ("Gruff", "Get out of my forge! Now!", 0.4, 1.0),
        ("Robot", "Greetings. Your package has arrived. Is that correct?", 0.5, 0.0),
    ];
    for (preset, text, mood, anger) in lines {
        let mut m = create("babble", preset);
        println!("dialogue {t:5.2} s  {preset}: {text}");
        t = talk(m.as_mut(), &mut out, t, text, mood, anger, 16.0);
    }
    out.truncate((t * SR) as usize);
    write(&format!("{dir}/dialogue.wav"), &out);

    // Creatures, small to big.
    let mut out = vec![0.0; (SR * 26.0) as usize];
    let calls: [(&str, &str, f32, f32, f32, f32); 9] = [
        ("creature_squeak", "", 0.0, 0.3, 0.8, 0.2),
        ("creature_chirp", "", 0.15, 0.2, 0.9, 1.6),
        ("creature_chatter", "Critter", 0.1, 0.4, 0.8, 2.9),
        ("creature_hiss", "Cat", 0.25, 0.8, 0.6, 4.4),
        ("creature_growl", "", 0.45, 0.6, 0.4, 6.1),
        ("creature_growl", "", 0.55, 0.9, 0.8, 8.3),
        ("creature_roar", "Big cat", 0.75, 0.6, 0.6, 10.6),
        ("creature_growl", "Bear", 0.8, 0.8, 0.3, 14.0),
        ("creature_roar", "Dragon", 1.0, 0.9, 0.7, 17.0),
    ];
    for (name, preset, size, aggr, exc, at) in calls {
        let mut m = create(name, preset);
        call(m.as_mut(), &mut out, at, size, aggr, exc, 1.0);
    }
    let end = out.iter().rposition(|s| s.abs() > 1e-4).unwrap_or(0) + 4800;
    out.truncate(end.min(out.len()));
    write(&format!("{dir}/creatures.wav"), &out);

    // A cat: resting breath, then a purr that swells.
    let mut m = create("creature_idle", "Cat");
    m.set_input_by_name("size", 0.25);
    m.set_input_by_name("excitement", 0.1);
    m.set_input_by_name("purr", 0.0);
    m.snap();
    let mut out = vec![0.0; (SR * 12.0) as usize];
    let (a, b) = out.split_at_mut((SR * 3.0) as usize);
    m.render_mono(a);
    m.set_input_by_name("purr", 1.0);
    m.render_mono(b);
    write(&format!("{dir}/purr.wav"), &out);
}
