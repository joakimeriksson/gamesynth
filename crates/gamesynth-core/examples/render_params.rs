//! Render any generator at fixed inputs and parameters to a stereo WAV: a command-line baker,
//! and the tool for fitting a model against a recording.
//!   cargo run -p gamesynth-core --release --example render_params -- piston out.wav 4 \
//!       preset="Muscle V8" @throttle=0.2 @rpm=0.4 engine/external_rpm=1 exhaust/muffling=0.3
//! `@name=value` sets an input, `group/name=value` a parameter, `preset=Name` loads a preset
//! first. Event generators are triggered once at the start. `script=file.csv` drives inputs
//! over time: a header row `t,<input>,<input>...`, then rows of seconds and values, linearly
//! interpolated (the render lasts until the last row unless <seconds> is shorter).
use gamesynth_core::generators;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: render_params <generator> <out.wav> <seconds> [preset=Name] [@input=v] [group/param=v] ...");
        std::process::exit(2);
    }
    let sr = 48000u32;
    let mut m = generators::create(&args[0], sr as f32).unwrap_or_else(|| panic!("unknown generator {}; have {:?}", args[0], generators::NAMES));
    let secs: f32 = args[2].parse().expect("seconds");
    let settings: Vec<(&str, &str)> = args[3..].iter().map(|a| a.split_once('=').unwrap_or_else(|| panic!("expected name=value, got {a}"))).collect();
    for (k, v) in settings.iter().filter(|(k, _)| *k == "preset") {
        let i = m.desc().preset_index(v).unwrap_or_else(|| panic!("no {k} {v}"));
        m.load_preset(i);
    }
    let mut script: Option<(Vec<String>, Vec<Vec<f32>>)> = None;
    for (_, path) in settings.iter().filter(|(k, _)| *k == "script") {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let names: Vec<String> = lines.next().expect("script header").split(',').skip(1).map(|n| n.trim().to_string()).collect();
        let rows: Vec<Vec<f32>> = lines.map(|l| l.split(',').map(|v| v.trim().parse().unwrap_or_else(|_| panic!("{path}: bad number in '{l}'"))).collect()).collect();
        assert!(rows.iter().all(|r| r.len() == names.len() + 1), "{path}: every row needs {} columns", names.len() + 1);
        script = Some((names, rows));
    }
    for (k, v) in settings.iter().filter(|(k, _)| *k != "preset" && *k != "script") {
        let v: f32 = v.parse().unwrap_or_else(|_| panic!("{k}: not a number: {v}"));
        let ok = match k.strip_prefix('@') {
            Some(input) => m.set_input_by_name(input, v),
            None => m.set_param_by_name(k, v),
        };
        assert!(ok, "{} has no {k}", args[0]);
    }
    // Inputs at time `t` from the script (linear between rows).
    let apply = |m: &mut dyn gamesynth_core::Model, t: f32| {
        if let Some((names, rows)) = &script {
            let i = rows.partition_point(|r| r[0] <= t).clamp(1, rows.len() - 1);
            let (a, b) = (&rows[i - 1], &rows[i]);
            let f = ((t - a[0]) / (b[0] - a[0]).max(1e-6)).clamp(0.0, 1.0);
            for (k, name) in names.iter().enumerate() {
                assert!(m.set_input_by_name(name, a[k + 1] + (b[k + 1] - a[k + 1]) * f), "no input {name}");
            }
        }
    };
    let secs = script.as_ref().map(|(_, rows)| secs.min(rows[rows.len() - 1][0])).unwrap_or(secs);
    apply(m.as_mut(), 0.0);
    m.snap();
    if m.desc().one_shot {
        m.trigger();
    }
    let n = (secs * sr as f32) as usize;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    for (i, (cl, cr)) in l.chunks_mut(480).zip(r.chunks_mut(480)).enumerate() {
        apply(m.as_mut(), i as f32 * 0.01);
        m.render_stereo(cl, cr);
    }
    let spec = hound::WavSpec { channels: 2, sample_rate: sr, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(&args[1], spec).expect("create wav");
    for (a, b) in l.iter().zip(&r) {
        w.write_sample((a.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
        w.write_sample((b.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
    }
    w.finalize().unwrap();
}
