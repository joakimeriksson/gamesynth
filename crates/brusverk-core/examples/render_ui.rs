//! Render the UI kit (`ui_*` generators) to stereo WAVs:
//!   cargo run -p brusverk-core --release --example render_ui -- kit <dir>
//!       every event in every style: <dir>/<style>_<event>.wav (+ a coin burst and a combo chain)
//!   cargo run -p brusverk-core --release --example render_ui -- demo <dir> [style ...]
//!       a menu-then-gameplay sequence per style (8 s): <dir>/demo_<style>.wav
//!   cargo run -p brusverk-core --release --example render_ui -- seq <out.wav> <secs> [param=v ...] <t>:<event>[@input=v ...] ...
//!       any timeline, e.g. `seq out.wav 3 preset=Glass music/key=9 0:ui_click 0.5:ui_coin@count=0.5`
//! Each event name gets its own instance (as a game gives each its own player), retriggered in
//! place, so a combo chain rings on one instance.
use brusverk_core::{generators, Model};
use std::collections::HashMap;

const SR: f32 = 48000.0;
const STYLES: [&str; 5] = ["Soft", "Wooden", "Glass", "Retro", "Sci-fi"];
const EVENTS: [&str; 19] = [
    "ui_hover", "ui_click", "ui_toggle_on", "ui_toggle_off", "ui_confirm", "ui_cancel", "ui_error", "ui_notify", "ui_open", "ui_close", "ui_slider",
    "ui_type", "ui_coin", "ui_collect", "ui_combo", "ui_level_up", "ui_countdown", "ui_go", "ui_fanfare",
];

struct Cue {
    t: f32,
    event: String,
    inputs: Vec<(String, f32)>,
}

fn file_tag(style: &str) -> String {
    style.to_lowercase().replace('-', "")
}

fn parse_cue(s: &str) -> Cue {
    let (t, rest) = s.split_once(':').unwrap_or_else(|| panic!("expected <t>:<event>, got {s}"));
    let mut parts = rest.split('@');
    let event = parts.next().unwrap().to_string();
    let inputs = parts
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or_else(|| panic!("expected input=value in {s}"));
            (k.to_string(), v.parse().unwrap_or_else(|_| panic!("bad number in {s}")))
        })
        .collect();
    Cue { t: t.parse().unwrap_or_else(|_| panic!("bad time in {s}")), event, inputs }
}

/// Render `cues` (sorted by time) for `secs` with the same params on every instance.
fn render(cues: &[Cue], secs: f32, params: &[(String, String)]) -> (Vec<f32>, Vec<f32>) {
    let mut models: HashMap<String, Box<dyn Model>> = HashMap::new();
    let n = (secs * SR) as usize;
    let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
    let (mut bl, mut br) = (vec![0.0f32; 256], vec![0.0f32; 256]);
    let mut next = 0;
    let mut pos = 0;
    while pos < n {
        // Fire every cue that falls in this block at its block start (5 ms resolution).
        while next < cues.len() && (cues[next].t * SR) as usize <= pos {
            let c = &cues[next];
            let m = models.entry(c.event.clone()).or_insert_with(|| {
                let mut m = generators::create(&c.event, SR).unwrap_or_else(|| panic!("no generator {}", c.event));
                for (k, v) in params.iter().filter(|(k, _)| k == "preset") {
                    let i = m.desc().preset_index(v).unwrap_or_else(|| panic!("no {k} {v}"));
                    m.load_preset(i);
                }
                for (k, v) in params.iter().filter(|(k, _)| k != "preset") {
                    assert!(m.set_param_by_name(k, v.parse().expect("number")), "{} has no {k}", c.event);
                }
                m
            });
            for (k, v) in &c.inputs {
                assert!(m.set_input_by_name(k, *v), "{} has no input {k}", c.event);
            }
            m.trigger();
            next += 1;
        }
        let len = (n - pos).min(240);
        for m in models.values_mut() {
            m.render_stereo(&mut bl[..len], &mut br[..len]);
            for i in 0..len {
                left[pos + i] += bl[i];
                right[pos + i] += br[i];
            }
        }
        pos += len;
    }
    (left, right)
}

fn write(path: &str, left: &[f32], right: &[f32]) {
    let spec = hound::WavSpec { channels: 2, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).unwrap_or_else(|e| panic!("{path}: {e}"));
    for (l, r) in left.iter().zip(right) {
        for s in [l, r] {
            w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
        }
    }
    w.finalize().unwrap();
}

/// The demo: menu (hover, hover, click, confirm, error), then play (a combo chain rising 1..8, a
/// coin burst, a level-up). Fits an 8 s clip.
fn demo_cues() -> Vec<Cue> {
    let mut c = vec![
        Cue { t: 0.05, event: "ui_hover".into(), inputs: vec![] },
        Cue { t: 0.4, event: "ui_hover".into(), inputs: vec![] },
        Cue { t: 0.75, event: "ui_click".into(), inputs: vec![] },
        Cue { t: 1.15, event: "ui_confirm".into(), inputs: vec![] },
        Cue { t: 1.8, event: "ui_error".into(), inputs: vec![] },
    ];
    for k in 0..8 {
        c.push(Cue { t: 2.6 + 0.24 * k as f32, event: "ui_combo".into(), inputs: vec![("combo".into(), k as f32 / 16.0)] });
    }
    c.push(Cue { t: 4.75, event: "ui_coin".into(), inputs: vec![("count".into(), 8.0 / 16.0)] });
    c.push(Cue { t: 5.85, event: "ui_level_up".into(), inputs: vec![] });
    c
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("kit") => {
            let dir = &a[1];
            std::fs::create_dir_all(dir).unwrap();
            for style in STYLES {
                let params = vec![("preset".to_string(), style.to_string())];
                for event in EVENTS {
                    let mut m = generators::create(event, SR).unwrap();
                    m.load_preset(m.desc().preset_index(style).unwrap());
                    let secs = m.length_secs().unwrap();
                    let (l, r) = render(&[Cue { t: 0.0, event: event.into(), inputs: vec![] }], secs, &params);
                    write(&format!("{dir}/{}_{event}.wav", file_tag(style)), &l, &r);
                }
                let burst = [Cue { t: 0.0, event: "ui_coin".into(), inputs: vec![("count".into(), 0.5)] }];
                let (l, r) = render(&burst, 2.0, &params);
                write(&format!("{dir}/{}_ui_coin_burst8.wav", file_tag(style)), &l, &r);
                let chain: Vec<Cue> = (0..8).map(|k| Cue { t: 0.24 * k as f32, event: "ui_combo".into(), inputs: vec![("combo".into(), k as f32 / 16.0)] }).collect();
                let (l, r) = render(&chain, 3.0, &params);
                write(&format!("{dir}/{}_ui_combo_chain.wav", file_tag(style)), &l, &r);
            }
            println!("{} files in {dir}", STYLES.len() * (EVENTS.len() + 2));
        }
        Some("demo") => {
            let dir = &a[1];
            std::fs::create_dir_all(dir).unwrap();
            let styles: Vec<&str> = if a.len() > 2 { a[2..].iter().map(String::as_str).collect() } else { STYLES.to_vec() };
            for style in styles {
                let (l, r) = render(&demo_cues(), 8.0, &[("preset".into(), style.into())]);
                let path = format!("{dir}/demo_{}.wav", file_tag(style));
                write(&path, &l, &r);
                println!("{path}");
            }
        }
        Some("seq") => {
            let (out, secs) = (&a[1], a[2].parse::<f32>().expect("seconds"));
            let mut params = Vec::new();
            let mut cues = Vec::new();
            for s in &a[3..] {
                match s.split_once('=') {
                    Some((k, v)) if !s.contains(':') => params.push((k.to_string(), v.to_string())),
                    _ => cues.push(parse_cue(s)),
                }
            }
            cues.sort_by(|x, y| x.t.total_cmp(&y.t));
            let (l, r) = render(&cues, secs, &params);
            write(out, &l, &r);
            println!("{out}");
        }
        _ => {
            eprintln!("usage: render_ui kit <dir> | demo <dir> [style ...] | seq <out.wav> <secs> [param=v ...] <t>:<event>[@input=v ...] ...");
            std::process::exit(2);
        }
    }
}
