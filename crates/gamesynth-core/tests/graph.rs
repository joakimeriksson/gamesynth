#![cfg(feature = "graph")]
use gamesynth_core::render::{peak, rms};
use gamesynth_core::{generators, GraphModel, Model};

const SR: f32 = 48000.0;

const FILES: &[(&str, &str)] = &[
    ("jet_lite", include_str!("../../../models/jet_lite.toml")),
    ("rain_on_tent", include_str!("../../../models/rain_on_tent.toml")),
    ("campfire", include_str!("../../../models/campfire.toml")),
    ("shield", include_str!("../../../models/shield.toml")),
    ("geiger", include_str!("../../../models/geiger.toml")),
    ("steam_vent", include_str!("../../../models/steam_vent.toml")),
    ("checkpoint", include_str!("../../../models/checkpoint.toml")),
    ("recharge", include_str!("../../../models/recharge.toml")),
];

fn render(m: &mut dyn Model, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (secs * SR) as usize];
    m.render_mono(&mut out);
    out
}

#[test]
fn shipped_models_compile_and_sound() {
    for (name, text) in FILES {
        let mut m = GraphModel::from_text(text, SR).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(m.desc().name, *name);
        assert_eq!(m.desc().engine, "graph");
        for preset in 0..m.desc().presets.len() {
            m.load_preset(preset);
            for i in 0..m.desc().inputs.len() {
                m.set_input(i, 1.0);
            }
            m.snap();
            if m.desc().one_shot {
                assert!(render(&mut m, 0.2).iter().all(|x| *x == 0.0), "{name} must be silent before its trigger");
                m.trigger();
            }
            let out = render(&mut m, 4.0);
            assert!(out.iter().all(|x| x.is_finite()), "{name} NaN");
            assert!(peak(&out) <= 1.0, "{name} peak {}", peak(&out));
            if m.desc().one_shot {
                assert!(peak(&out) > 0.1, "{name} one-shot too quiet: {}", peak(&out));
                assert!(m.is_finished(), "{name} one-shot never finished");
                let again = {
                    m.trigger();
                    render(&mut m, 4.0)
                };
                assert!(out.iter().zip(&again).any(|(a, b)| (a - b).abs() > 1e-3), "{name}: `rnd` should vary each trigger");
            } else {
                assert!(rms(&out[SR as usize..]) > 0.02, "{name} preset {preset} too quiet: {}", rms(&out[SR as usize..]));
            }
        }
    }
}

#[test]
fn file_order_is_kept_and_names_are_grouped() {
    let m = GraphModel::from_text(FILES[0].1, SR).unwrap();
    let inputs: Vec<_> = m.desc().inputs.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(inputs, ["throttle", "boost", "speed"]);
    assert_eq!(m.desc().params[0].name, "spool/spool_up");
    assert_eq!(m.desc().params.last().unwrap().name, "master/gain");
    assert_eq!(m.desc().presets[1].name, "heavy");
    assert_eq!(m.desc().presets[1].values[m.desc().param_index("whine/whine_hz").unwrap()], 1400.0);
}

#[test]
fn graph_jet_spools_like_the_native_one() {
    let mut g = GraphModel::from_text(FILES[0].1, SR).unwrap();
    let mut n = generators::create("jet", SR).unwrap();
    assert!((g.signal("rpm").unwrap() - 0.0).abs() < 1e-6);
    render(&mut g, 0.01);
    assert!((g.signal("rpm").unwrap() - 0.28).abs() < 1e-3, "starts at idle");
    g.set_input(0, 1.0);
    n.set_input(0, 1.0);
    render(&mut g, 0.1);
    assert!(g.signal("rpm").unwrap() < 0.5, "rpm must not jump");
    render(&mut g, 1.1);
    let rpm = g.signal("rpm").unwrap();
    assert!(rpm > 0.93 && rpm < 1.0, "rpm after spool_up: {rpm}");
    // Same ballpark loudness as the hand-written engine at full throttle.
    render(n.as_mut(), 1.2);
    let (a, b) = (rms(&render(&mut g, 2.0)), rms(&render(n.as_mut(), 2.0)));
    assert!(a > b * 0.5 && a < b * 2.0, "graph {a} vs native {b}");
}

#[test]
fn params_inputs_and_json() {
    let mut m = GraphModel::from_text(FILES[4].1, SR).unwrap();
    let i = m.desc().param_index("clicks/max_rate").unwrap();
    m.set_param(i, 1e9);
    assert_eq!(m.param(i), 3000.0);
    m.set_input(0, 5.0);
    assert_eq!(m.input(0), 1.0);
    let quiet = {
        let mut q = GraphModel::from_text(FILES[4].1, SR).unwrap();
        q.set_input(0, 0.0);
        q.snap();
        rms(&render(&mut q, 2.0))
    };
    m.snap();
    assert!(rms(&render(&mut m, 2.0)) > quiet * 3.0);

    let json = r#"{ "model": {"name": "beep"}, "inputs": {"on": {"default": 1}},
        "params": {"hz": {"default": 440, "min": 100, "max": 2000, "scale": "exp"}},
        "graph": {"nodes": [{"id": "o", "type": "sine", "freq": "hz"}, {"id": "g", "type": "gain", "in": ["o"], "gain": "on * 0.5"}], "out": "g"} }"#;
    let mut beep = GraphModel::from_text(json, SR).unwrap();
    let out = render(&mut beep, 0.5);
    assert!((peak(&out) - 0.5).abs() < 0.01);
    let crossings = out.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    assert!((crossings as i32 - 440).abs() <= 2, "440 Hz for 0.5 s: {crossings}");
}

#[test]
fn bad_files_are_rejected_with_useful_messages() {
    let base = |nodes: &str, extra: &str| format!("{extra}\n[graph]\nnodes = [{nodes}]\nout = \"a\"\n");
    let cases: &[(&str, String)] = &[
        ("unknown type 'wobble'", base(r#"{ id = "a", type = "wobble" }"#, "")),
        ("unknown argument 'cutof'", base(r#"{ id = "n", type = "noise" }, { id = "a", type = "svf", in = ["n"], cutof = 100 }"#, "")),
        ("missing required 'cutoff'", base(r#"{ id = "n", type = "noise" }, { id = "a", type = "svf", in = ["n"] }"#, "")),
        ("unknown name 'trottle'", base(r#"{ id = "a", type = "sine", freq = "trottle * 100" }"#, "[inputs]\nthrottle = {}")),
        ("'zz' is not a node id", base(r#"{ id = "a", type = "gain", gain = 1, in = ["zz"] }"#, "")),
        ("form a loop", base(r#"{ id = "a", type = "gain", gain = 1, in = ["b"] }, { id = "b", type = "gain", gain = 1, in = ["a"] }"#, "")),
        ("duplicate node id", base(r#"{ id = "a", type = "noise" }, { id = "a", type = "noise" }"#, "")),
        ("needs at least one input", base(r#"{ id = "a", type = "limiter" }"#, "")),
        ("takes no `in`", base(r#"{ id = "n", type = "noise" }, { id = "a", type = "sine", freq = 1, in = ["n"] }"#, "")),
        ("already defined", base(r#"{ id = "a", type = "noise" }"#, "[inputs]\nx = {}\n[signals]\nx = \"1\"")),
        ("reserved word", base(r#"{ id = "a", type = "noise" }"#, "[inputs]\nlerp = {}")),
        ("signals form a loop", base(r#"{ id = "a", type = "noise" }"#, "[signals]\np = \"q + 1\"\nq = \"p * 2\"")),
        ("default within the range", base(r#"{ id = "a", type = "noise" }"#, "[params]\ng = { default = 5, min = 0, max = 1 }")),
        ("unknown param 'nope'", base(r#"{ id = "a", type = "noise" }"#, "[presets.x]\nnope = 1")),
        ("TOML", "this is not toml [".to_string()),
    ];
    for (expect, text) in cases {
        match GraphModel::from_text(text, SR) {
            Ok(_) => panic!("accepted a bad file; expected error containing \"{expect}\""),
            Err(e) => assert!(e.to_string().contains(expect), "expected \"{expect}\" in: {e}"),
        }
    }
    let many: String = (0..300).map(|i| format!("{{ id = \"n{i}\", type = \"noise\" }},")).collect();
    assert!(GraphModel::from_text(&format!("[graph]\nnodes = [{many}]\nout = \"n0\""), SR).unwrap_err().to_string().contains("between 1 and"));
}

#[test]
fn signals_may_be_declared_in_any_order() {
    let text = "[inputs]\nx = { default = 1 }\n[signals]\nb = \"a * 2\"\na = \"x + 1\"\n[graph]\nnodes = [{ id = \"o\", type = \"dc\", value = \"b\" }]\nout = \"o\"";
    let mut m = GraphModel::from_text(text, SR).unwrap();
    render(&mut m, 0.01);
    assert_eq!(m.signal("b"), Some(4.0));
}

#[test]
fn graph_cost_relative_to_native() {
    let mut g = GraphModel::from_text(FILES[0].1, SR).unwrap();
    let mut n = generators::create("jet", SR).unwrap();
    g.set_input(0, 0.8);
    n.set_input(0, 0.8);
    let time = |m: &mut dyn Model| {
        let mut buf = vec![0.0; 512];
        let start = std::time::Instant::now();
        for _ in 0..(4 * SR as usize / 512) {
            m.render_mono(&mut buf);
        }
        start.elapsed().as_secs_f64()
    };
    let (tg, tn) = (time(&mut g), time(n.as_mut()));
    println!("graph jet_lite ({} nodes): {:.1}x real time, native jet: {:.1}x real time, ratio {:.2}", g.node_count(), 4.0 / tg, 4.0 / tn, tg / tn);
    assert!(tg < 4.0 * 0.25, "graph model must stay well under real time: {tg}s for 4 s");
}
