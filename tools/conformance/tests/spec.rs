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

/// History as the app opens it, from the merged defaults.
fn limits() -> history::Limits {
    config::Config::default().history_limits()
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
        let mut h = history::History::open(
            None,
            history::Limits {
                ram_minutes: ram_min,
                ..limits()
            },
        );
        for r in v["results"].as_array().unwrap() {
            h.record_probe(&model::ProbeResult {
                probe: probe.clone(),
                at: r["at"].as_u64().unwrap(),
                rtt_ms: opt(&r["rtt_ms"]),
            });
        }
        assert_eq!(
            v["window_s"].as_u64().unwrap(),
            limits().recent_window_s,
            "probe-stats/{name} window"
        );
        assert_eq!(
            stat_json(&h.now(&probe)),
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
            limits().hop_window,
            "hop-trace/{name}"
        );
        let mut h = history::History::open(None, limits());
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

/// The Default preset in each Appearance against the spec's tokens: the base file is dark;
/// light replaces each color its file has (spec `tokens/README.md`).
#[test]
fn palette_matches_the_tokens() {
    use model::Section;
    use palette::{Palette, Preset, Shade};
    let base = load("tokens/blinkengraphs.tokens.json");
    let light = load("tokens/blinkengraphs.light.tokens.json");
    for shade in [Shade::Dark, Shade::Light] {
        let p = Palette::new(Preset::Default, shade, &Default::default());
        let tok = |path: &str| {
            let find = |t: &Value| {
                path.split('.').fold(&t["color"], |v, k| &v[k])["$value"]
                    .as_str()
                    .map(str::to_string)
            };
            (shade == Shade::Light)
                .then(|| find(&light))
                .flatten()
                .or_else(|| find(&base))
                .unwrap()
        };
        let hex = |c: palette::Rgb| String::from(c);
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
                "{shade:?} section.{k}"
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
            assert_eq!(hex(c), tok(k), "{shade:?} {k}");
        }
    }
    let th = |k: &str| base["threshold"][k]["$value"].as_f64().unwrap() as f32;
    assert_eq!(palette::METER_WARN, th("meter-warn"));
    assert_eq!(palette::METER_CRIT, th("meter-crit"));
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

#[test]
fn history_timing() {
    use std::time::Duration;
    for c in load("vectors/history/timing.json")["cases"]
        .as_array()
        .unwrap()
    {
        let start = c["start"].as_f64().unwrap();
        let interval = Duration::from_secs_f64(c["interval_s"].as_f64().unwrap());
        let (mut due, mut end) = (0.0f64, 0.0f64);
        for (d, want) in c["durations_s"]
            .as_array()
            .unwrap()
            .iter()
            .zip(c["expected"].as_array().unwrap())
        {
            let begin = due.max(end);
            let at = (start + begin).floor() as u64;
            assert!(
                (start + begin - want["start"].as_f64().unwrap()).abs() < 1e-6,
                "{}: round at {begin}",
                c["name"]
            );
            assert_eq!(at, want["at"].as_u64().unwrap(), "{}", c["name"]);
            end = begin + d.as_f64().unwrap();
            due = probe::next_round(interval, Duration::from_secs_f64(begin)).as_secs_f64();
        }
    }
}

#[test]
fn history_minutes() {
    let probe = icmp_1111();
    let key = history::Key::Probe(probe.clone());
    for (i, c) in load("vectors/history/minutes.json")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let db = std::env::temp_dir().join(format!(
            "bg-conformance-minutes-{}-{i}.db",
            std::process::id()
        ));
        let _ = fs::remove_file(&db);
        let timeout = c["max_timeout_ms"].as_u64().unwrap();
        let mut h = history::History::open(
            Some(&db),
            history::Limits {
                minute_grace_s: timeout.div_ceil(1000) + 1,
                ..limits()
            },
        );
        let mut seen = std::collections::BTreeSet::new();
        for (at, want) in c["arrivals"]
            .as_array()
            .unwrap()
            .iter()
            .zip(c["expected"].as_array().unwrap())
        {
            h.record_probe(&model::ProbeResult {
                probe: probe.clone(),
                at: at.as_u64().unwrap(),
                rtt_ms: Some(1.0),
            });
            let now: Vec<u64> = h
                .minutes(&key, 0, u64::MAX / 2)
                .into_iter()
                .map(|(m, _)| m)
                .filter(|m| seen.insert(*m))
                .collect();
            assert_eq!(json!(now), *want, "{} after {at}", c["name"]);
        }
        // Every arrival in a written minute, late ones included, is counted there.
        let stored = h.minutes(&key, 0, u64::MAX / 2);
        let n: u64 = stored.iter().map(|(_, m)| m.n).sum();
        let due = c["arrivals"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| {
                stored
                    .iter()
                    .any(|(m, _)| a.as_u64().unwrap() / 60 * 60 == *m)
            })
            .count();
        assert_eq!(n as usize, due, "{}", c["name"]);
        drop(h);
        let _ = fs::remove_file(&db);
    }
}

#[test]
fn history_ranges() {
    for r in load("vectors/history/ranges.json")["ranges"]
        .as_array()
        .unwrap()
    {
        let w = history::bucket_width(r["span_s"].as_u64().unwrap());
        assert_eq!(w, r["ram_bucket_s"].as_u64().unwrap(), "{}", r["range"]);
        assert_eq!(
            w.div_ceil(60) * 60,
            r["storage_bucket_s"].as_u64().unwrap(),
            "{}",
            r["range"]
        );
    }
}

#[test]
fn history_buckets() {
    let v = load("vectors/history/buckets.json");
    let ram: Vec<(u64, Option<f32>)> = v["ram"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["at"].as_u64().unwrap(), opt(&r["v"])))
        .collect();
    let stored: Vec<(u64, history::Minute)> = v["stored"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let mut vals =
                vec![m["max"].as_f64().map(|x| x as f32); m["n"].as_u64().unwrap() as usize];
            vals.extend(std::iter::repeat_n(
                None,
                m["lost"].as_u64().unwrap() as usize,
            ));
            (m["m"].as_u64().unwrap(), history::Minute::of(&vals))
        })
        .collect();
    let written = v["written_through"].as_u64().unwrap();
    for c in v["cases"].as_array().unwrap() {
        let (now, span) = (c["now"].as_u64().unwrap(), c["span_s"].as_u64().unwrap());
        let pts = history::buckets(
            now - span,
            now,
            history::bucket_width(span),
            written,
            ram.iter().copied(),
            &stored,
            model::Aggregate::Max,
        );
        let got: Vec<Value> = pts
            .iter()
            .map(|p| {
                json!({"at": p.at, "max": (!p.value.is_nan()).then_some(p.value as f64), "loss": r2(p.loss as f64)})
            })
            .collect();
        assert_eq!(json!(got), c["expected"], "buckets {}", c["range"]);
    }
}
