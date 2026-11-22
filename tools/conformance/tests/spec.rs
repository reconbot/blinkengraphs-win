use model::{Metric, Probe, ProbeStat, Sample};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

const NOW: u64 = 1_800_000_000;

fn spec(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec")
        .join(rel)
}

fn load(rel: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(spec(rel)).unwrap()).unwrap()
}

/// Every vector in a folder, by file name.
fn vectors(dir: &str) -> Vec<(String, Value)> {
    let mut out: Vec<_> = fs::read_dir(spec(&format!("vectors/{dir}")))
        .expect("spec/ missing: run git submodule update --init")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .map(|p| {
            let v = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
            (p.file_name().unwrap().to_string_lossy().into_owned(), v)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!out.is_empty(), "no vectors in spec/vectors/{dir}");
    out
}

/// Two decimals, as the vectors carry them.
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn opt(v: &Value) -> Option<f32> {
    v.as_f64().map(|x| x as f32)
}

fn icmp_1111() -> Probe {
    serde_json::from_value(json!({"protocol": "icmp", "target": "1.1.1.1"})).unwrap()
}

fn stat(v: &Value) -> ProbeStat {
    ProbeStat {
        loss: v["loss"].as_f64().unwrap() as f32,
        rtt_ms: opt(&v["rtt_ms"]),
        baseline_ms: opt(&v["baseline_ms"]),
        jitter_ms: opt(&v["jitter_ms"]),
    }
}

fn r2o(v: Option<f32>) -> Option<f64> {
    v.map(|x| r2(x as f64))
}

fn stat_json(st: &ProbeStat) -> Value {
    json!({
        "loss": r2(st.loss as f64),
        "rtt_ms": r2o(st.rtt_ms),
        "baseline_ms": r2o(st.baseline_ms),
        "jitter_ms": r2o(st.jitter_ms),
    })
}

#[test]
fn verdict() {
    for (name, v) in vectors("verdict") {
        let i = &v["input"];
        let sample = i["sample"].as_object().map(|m| {
            let mut s = Sample {
                at: NOW,
                values: [0.0; Metric::ALL.len()],
                per_core: vec![],
            };
            for metric in Metric::ALL {
                if let Some(x) = m.get(&format!("{metric:?}")).and_then(Value::as_f64) {
                    s.set(metric, x);
                }
            }
            s
        });
        let probes: Vec<(Probe, ProbeStat)> = i["probes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    serde_json::from_value(p["probe"].clone()).unwrap(),
                    stat(&p["stat"]),
                )
            })
            .collect();
        let got = verdict::diagnose(&verdict::Input {
            sample: sample.as_ref(),
            probes: &probes,
            upload_peak_bps: i["upload_peak_bps"].as_f64().unwrap_or(0.0),
            path_up: i["path_up"].as_bool().unwrap_or(true),
            via_vpn: i["via_vpn"].as_bool().unwrap_or(false),
            captive_portal: i["captive_portal"].as_bool().unwrap_or(false),
        });
        assert_eq!(json!(got), v["expected"], "verdict/{name}");
    }
}

#[test]
fn probe_stats() {
    let probe = icmp_1111();
    for (name, v) in vectors("probe-stats") {
        let ram_min = (v["ram_s"].as_u64().unwrap() / 60) as u32;
        let mut h = history::History::open(None, ram_min, 7);
        for r in v["results"].as_array().unwrap() {
            h.record_probe(&model::ProbeResult {
                probe: probe.clone(),
                at: r["at"].as_u64().unwrap(),
                rtt_ms: opt(&r["rtt_ms"]),
            });
        }
        let window = v["window_s"].as_u64().unwrap();
        assert_eq!(
            stat_json(&h.stat(&probe, window)),
            v["expected"],
            "probe-stats/{name}"
        );
    }
}

#[test]
fn hop_trace() {
    let trace = icmp_1111();
    for (name, v) in vectors("hop-trace") {
        assert_eq!(
            v["hop_window"].as_u64().unwrap() as usize,
            history::HOP_WINDOW,
            "hop-trace/{name}"
        );
        let mut h = history::History::open(None, 15, 7);
        for (i, tick) in v["ticks"].as_array().unwrap().iter().enumerate() {
            for hop in tick.as_array().unwrap() {
                h.record_hop(&model::HopResult {
                    trace: trace.clone(),
                    at: NOW + i as u64,
                    ttl: hop["ttl"].as_u64().unwrap() as u8,
                    addr: hop["addr"].as_str().map(|a| a.parse().unwrap()),
                    rtt_ms: opt(&hop["rtt_ms"]),
                    reached: hop["reached"].as_bool().unwrap(),
                });
            }
        }
        let got: Vec<Value> = h
            .hops(&trace)
            .iter()
            .map(|x| {
                json!({
                    "ttl": x.ttl, "addr": x.addr, "sent": x.sent, "loss": r2(x.loss as f64),
                    "last_ms": r2o(x.last_ms), "best_ms": r2o(x.best_ms),
                    "avg_ms": r2o(x.avg_ms), "worst_ms": r2o(x.worst_ms),
                })
            })
            .collect();
        assert_eq!(json!(got), v["expected"], "hop-trace/{name}");
    }
}

#[test]
fn meters() {
    for p in load("vectors/meters/cube-root.json")["points"]
        .as_array()
        .unwrap()
    {
        let got = render::cube_root(p["value"].as_f64().unwrap(), p["max"].as_f64().unwrap());
        let want = p["normalized"].as_f64().unwrap();
        assert!((got as f64 - want).abs() < 1e-6, "cube-root {p}: got {got}");
    }
    for p in load("vectors/meters/latency-scale.json")["points"]
        .as_array()
        .unwrap()
    {
        let scale = meters::latency_scale(opt(&p["baseline_ms"]));
        assert_eq!(
            scale as f64,
            p["scale_ms"].as_f64().unwrap(),
            "latency-scale {p}"
        );
        for n in p["normalized"].as_array().unwrap() {
            let rtt = n["rtt_ms"].as_f64().unwrap() as f32;
            assert_eq!(
                r2((rtt / scale).min(1.0) as f64),
                n["value"].as_f64().unwrap(),
                "latency-scale {n}"
            );
        }
    }
}

fn bits(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())
}

/// The exact value is `bits`; `value` is a readable label that serde_json can parse an ULP off.
fn exact(mut v: Value) -> Value {
    match &mut v {
        Value::Object(o) if o.contains_key("bits") => {
            o.remove("value");
        }
        Value::Object(o) => o.values_mut().for_each(|x| *x = exact(x.take())),
        Value::Array(a) => a.iter_mut().for_each(|x| *x = exact(x.take())),
        _ => {}
    }
    v
}

fn bits_json(v: f64) -> Value {
    json!({"value": v, "bits": format!("{:016x}", v.to_bits())})
}

fn minute_json(m: &history::Minute) -> Value {
    json!({
        "n": m.n, "lost": m.lost,
        "sum": (m.n > 0).then(|| bits_json(m.sum)),
        "min": (m.n > 0).then(|| bits_json(m.min)),
        "max": (m.n > 0).then(|| bits_json(m.max)),
        "sk": m.sketch.encode().iter().map(|b| format!("{b:02x}")).collect::<String>(),
    })
}

#[test]
fn sketch() {
    for c in load("vectors/sketch/index.json")["cases"]
        .as_array()
        .unwrap()
    {
        let v = bits(&c["value"]);
        let i = history::sketch_index(v);
        assert_eq!(json!(i), c["index"], "sketch index of {v}");
        assert_eq!(
            exact(json!(i.map(|i| bits_json(history::sketch_rep(i))))),
            exact(c["rep"].clone()),
            "sketch rep of {v}"
        );
    }
    let minutes: Vec<history::Minute> = load("vectors/sketch/minutes.json")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let values: Vec<Option<f32>> =
                c["values"].as_array().unwrap().iter().map(opt).collect();
            let m = history::Minute::of(&values);
            assert_eq!(
                exact(minute_json(&m)),
                exact(c["minute"].clone()),
                "sketch minute of {values:?}"
            );
            m
        })
        .collect();
    let merge = load("vectors/sketch/merge.json");
    let mut acc = history::Minute::default();
    for m in &minutes {
        acc.merge(m);
    }
    assert_eq!(
        exact(minute_json(&acc)),
        exact(merge["merged"].clone()),
        "sketch merge"
    );
    assert_eq!(
        r2(acc.loss() as f64),
        merge["loss"].as_f64().unwrap(),
        "sketch merge loss"
    );
    for a in model::Aggregate::ALL {
        assert_eq!(
            exact(acc.value(a).map(bits_json).unwrap_or(Value::Null)),
            exact(merge["aggregates"][a.label()].clone()),
            "sketch aggregate {}",
            a.label()
        );
    }
}

#[test]
fn defaults_merge() {
    for c in load("vectors/defaults/merge.json")["cases"]
        .as_array()
        .unwrap()
    {
        let got = config::defaults::merge(c["base"].clone(), c["override"].clone());
        assert_eq!(got, c["expected"], "defaults merge: {}", c["name"]);
    }
}

#[test]
fn public_address() {
    use probe::public::{Reading, Source, read};
    for (name, v) in vectors("public-address") {
        let i = &v["input"];
        let txt: Option<Vec<String>> = i["dns_txt"]
            .as_array()
            .map(|a| a.iter().map(|s| s.as_str().unwrap().to_string()).collect());
        let with_asn = |asn: Option<String>, mut o: Value| {
            if let Some(a) = asn {
                o["asn"] = json!(a);
            }
            o
        };
        let got = match read(
            i["family"] == "ipv6",
            txt.as_deref(),
            i["https_body"].as_str(),
        ) {
            Reading::None => json!({"state": "none"}),
            Reading::Nat64(a) => json!({"state": "nat64", "address": a.to_string()}),
            Reading::Agree { address, asn } => with_asn(
                asn,
                json!({"state": "agree", "address": address.to_string()}),
            ),
            Reading::Differ { dns, https, asn } => with_asn(
                asn,
                json!({"state": "differ", "dns": dns.to_string(), "https": https.to_string()}),
            ),
            Reading::Single {
                address,
                source,
                asn,
            } => with_asn(
                asn,
                json!({"state": "single", "address": address.to_string(),
                       "source": if source == Source::Dns { "dns" } else { "https" }}),
            ),
        };
        assert_eq!(got, v["expected"], "public-address/{name}");
    }
}

/// These defaults are still compile-time constants in the engine crates, so an
/// override of them takes effect only once those crates read `config::defaults`.
#[test]
fn engine_constants_match_the_merged_defaults() {
    let d = config::defaults::get();
    assert_eq!(probe::MAX_TIMEOUT.as_millis() as u64, d.max_timeout_ms);
    assert_eq!(probe::QUERY_NAME, d.dns_query_name);
    assert_eq!(probe::MAX_TTL, d.hop_trace.max_ttl);
    assert_eq!(
        probe::FULL_SWEEP_EVERY.as_secs(),
        d.hop_trace.full_sweep_every_s
    );
    assert_eq!(history::HOP_WINDOW, d.hop_trace.hop_window);
    assert_eq!(history::RECENT_SECS, d.history.recent_window_s);
}

#[test]
fn dark_palette_matches_the_base_tokens() {
    use model::Section;
    use palette::{Palette, Preset, Shade};
    let t = load("tokens/blinkengraphs.tokens.json");
    let p = Palette::new(Preset::Default, Shade::Dark, &Default::default());
    let hex = |c: palette::Rgb| String::from(c);
    let tok = |path: &str| {
        path.split('.').fold(&t["color"], |v, k| &v[k])["$value"]
            .as_str()
            .unwrap()
            .to_string()
    };
    for (s, k) in [
        (Section::Cpu, "cpu"),
        (Section::Memory, "memory"),
        (Section::Network, "network"),
        (Section::Disk, "disk"),
        (Section::Latency, "latency"),
    ] {
        assert_eq!(
            hex(p.section(s)),
            tok(&format!("section.{k}")),
            "section.{k}"
        );
    }
    for (c, k) in [
        (p.upload, "section.upload"),
        (p.loss, "loss"),
        (p.ok, "severity.ok"),
        (p.warn, "severity.warn"),
        (p.crit, "severity.crit"),
        (p.bg, "surface.background"),
        (p.card, "surface.card"),
        (p.card_edge, "surface.card-edge"),
        (p.text, "surface.text"),
        (p.text_weak, "surface.text-weak"),
    ] {
        assert_eq!(hex(c), tok(k), "{k}");
    }
    let th = |k: &str| t["threshold"][k]["$value"].as_f64().unwrap() as f32;
    assert_eq!(render::WARN, th("meter-warn"));
    assert_eq!(render::CRIT, th("meter-crit"));
}

#[test]
fn fixtures_match_the_spec_scenarios() {
    for s in fixtures::all() {
        let want = load(&format!("fixtures/scenarios/{}.json", s.name));
        let got = json!({
            "name": s.name, "caption": s.caption, "interval_s": 1, "normalized": true,
            "cpu": s.cpu, "memory": s.memory, "disk": s.disk, "down": s.down, "up": s.up, "rtt": s.rtt,
        });
        // f32 series round-trip through JSON as f64, so compare after the same trip.
        let got: Value = serde_json::from_str(&got.to_string()).unwrap();
        assert_eq!(got, want, "fixtures/scenarios/{}", s.name);
    }
}
