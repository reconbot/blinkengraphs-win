use history::Row;
use history::archive::Archive;
use history::pause::Pause;
use history::range::{GraphPoint, Range, Source, Sources, State};
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
                ..Sample::default()
            };
            for metric in Metric::ALL {
                if let Some(x) = m.get(&format!("{metric:?}")).and_then(Value::as_f64) {
                    s.set(metric, x);
                }
            }
            if let Some(d) = m.get("Disks") {
                s.disks = serde_json::from_value(d.clone()).unwrap();
            }
            if let Some(p) = m.get("MemPressure") {
                s.pressure = serde_json::from_value(p.clone()).unwrap();
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
            since_pause_s: i["since_pause_s"].as_u64(),
            window_s: limits().recent_window_s,
        });
        assert_eq!(json!(got), v["expected"], "verdict/{name}");
    }
}

#[test]
fn probe_stats() {
    for (name, v) in vectors("probe-stats") {
        let results: Vec<(u64, Option<f32>)> = v["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["at"].as_u64().unwrap(), opt(&r["rtt_ms"])))
            .collect();
        let window_s = v["window_s"].as_u64().unwrap();
        assert_eq!(
            window_s,
            limits().recent_window_s,
            "probe-stats/{name} window"
        );
        let got = history::probe_stats(
            &results,
            v["now"].as_u64().unwrap(),
            window_s,
            v["raw_s"].as_u64().unwrap(),
        );
        let want = &v["expected"];
        match got {
            None => assert!(want.is_null(), "probe-stats/{name}: no stats, want {want}"),
            Some(st) => {
                let got = stat_json(&st);
                for k in ["loss", "rtt_ms", "baseline_ms", "jitter_ms"] {
                    let close = match (got[k].as_f64(), want[k].as_f64()) {
                        (Some(a), Some(b)) => (a - b).abs() <= 0.01,
                        (a, b) => a.is_none() && b.is_none(),
                    };
                    assert!(close, "probe-stats/{name} {k}: got {got}, want {want}");
                }
            }
        }
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

fn row_json(m: &history::Row) -> Value {
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
    let minutes: Vec<history::Row> = load("vectors/sketch/minutes.json")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let values: Vec<Option<f32>> =
                c["values"].as_array().unwrap().iter().map(opt).collect();
            let m = history::Row::of(&values);
            assert_eq!(
                exact(row_json(&m)),
                exact(c["minute"].clone()),
                "sketch minute of {values:?}"
            );
            m
        })
        .collect();
    let merge = load("vectors/sketch/merge.json");
    let mut acc = history::Row::default();
    for m in &minutes {
        acc.merge(m);
    }
    assert_eq!(
        exact(row_json(&acc)),
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

fn results(v: &Value) -> Vec<(u64, Option<f32>)> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| (r["at"].as_u64().unwrap(), opt(&r["rtt_ms"])))
        .collect()
}

fn pauses(v: &Value) -> Vec<Pause> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|p| Pause {
                    begin: p["begin"].as_f64().unwrap(),
                    end: p["end"].as_f64().unwrap(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn paused_json(a: &Archive) -> Value {
    json!(
        a.paused
            .iter()
            .map(|(m, s)| json!({"m": m, "paused_s": s}))
            .collect::<Vec<_>>()
    )
}

fn rows_json(a: &Archive) -> Value {
    json!(
        a.rows
            .iter()
            .map(|(m, r)| {
                let mut j = row_json(r);
                j["m"] = json!(m);
                j
            })
            .collect::<Vec<_>>()
    )
}

/// An Archive given as Rows built from their values, plus Paused seconds per step.
fn archive(step: u64, rows: &Value, paused: &Value) -> Archive {
    let rows: Vec<(u64, Row)> = rows.as_array().map_or(vec![], |rs| {
        rs.iter()
            .map(|r| {
                let vs: Vec<Option<f32>> =
                    r["values"].as_array().unwrap().iter().map(opt).collect();
                (r["m"].as_u64().unwrap(), Row::of(&vs))
            })
            .collect()
    });
    let paused = paused.as_array().map_or(vec![], |ps| {
        ps.iter()
            .map(|p| (p["m"].as_u64().unwrap(), p["paused_s"].as_f64().unwrap()))
            .collect()
    });
    let mut a = Archive { step, rows, paused };
    // Vectors list Rows in any order; an Archive keeps them ascending.
    a.rows.sort_by_key(|r| r.0);
    a.paused.sort_by_key(|p| p.0);
    a
}

#[test]
fn history_rounds() {
    for c in load("vectors/history/rounds.json")["cases"]
        .as_array()
        .unwrap()
    {
        let ps = pauses(&c["pauses"]);
        let recorded: Vec<u64> = c["rounds"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| {
                !history::pause::cut(
                    r["start"].as_f64().unwrap(),
                    r["end"].as_f64().unwrap(),
                    &ps,
                )
            })
            .map(|r| r["start"].as_f64().unwrap().floor() as u64)
            .collect();
        assert_eq!(
            json!(recorded),
            c["expected"],
            "history/rounds {}",
            c["name"]
        );
    }
}

#[test]
fn history_raw() {
    let v = load("vectors/history/raw.json");
    for c in v["retention"].as_array().unwrap() {
        let (now, raw_s) = (c["now"].as_u64().unwrap(), c["raw_s"].as_u64().unwrap());
        let kept: Vec<u64> = c["ats"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_u64().unwrap())
            .filter(|at| history::pause::raw_keeps(*at, now, raw_s))
            .collect();
        assert_eq!(json!(kept), c["expected"], "history/raw {}", c["name"]);
    }
    for c in v["kill"].as_array().unwrap() {
        let p = Pause::at_launch(
            c["newest_at"].as_u64(),
            c["launch"].as_f64().unwrap(),
            c["settle_s"].as_f64().unwrap(),
        );
        assert_eq!(
            json!({"begin": p.begin, "end": p.end}),
            c["expected"],
            "history/raw {}",
            c["name"]
        );
    }
}

#[test]
fn history_launch() {
    for c in load("vectors/history/launch.json")["cases"]
        .as_array()
        .unwrap()
    {
        let newest = c["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["at"].as_u64().unwrap())
            .max();
        let p = Pause::at_launch(
            newest,
            c["launch"].as_f64().unwrap(),
            c["settle_s"].as_f64().unwrap(),
        );
        assert_eq!(
            json!({"begin": p.begin, "end": p.end}),
            c["expected"],
            "history/launch {}",
            c["name"]
        );
    }
}

#[test]
fn history_link_settle() {
    for c in load("vectors/history/link-settle.json")["cases"]
        .as_array()
        .unwrap()
    {
        let changes: Vec<f64> = c["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect();
        let settles = history::pause::link_settle(&changes, c["settle_s"].as_f64().unwrap());
        let recorded: Vec<u64> = c["rounds"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| {
                !history::pause::cut(
                    r["start"].as_f64().unwrap(),
                    r["end"].as_f64().unwrap(),
                    &settles,
                )
            })
            .map(|r| r["start"].as_f64().unwrap().floor() as u64)
            .collect();
        assert_eq!(
            json!(recorded),
            c["expected"],
            "history/link-settle {}",
            c["name"]
        );
    }
}

#[test]
fn history_rows() {
    let v = load("vectors/history/rows.json");
    let g = history::archive::grace_s(v["max_timeout_ms"].as_u64().unwrap());
    let (res, ps) = (results(&v["results"]), pauses(&v["pauses"]));
    let step = v["step_s"].as_u64().unwrap();
    for c in v["checks"].as_array().unwrap() {
        let now = c["now"].as_u64().unwrap();
        let a1 = Archive::from_raw(&res, &ps, step, now, g);
        assert_eq!(
            exact(rows_json(&a1)),
            exact(c["rows"].clone()),
            "history/rows at {now}"
        );
        assert_eq!(
            paused_json(&a1),
            c["paused"],
            "history/rows paused at {now}"
        );
    }
}

#[test]
fn history_archives() {
    let v = load("vectors/history/archives.json");
    let steps: Vec<u64> = limits_archives().iter().map(|a| a.step_s).collect();
    let a1 = archive(steps[0], &v["a1"], &v["a1_paused"]);
    let a2 = a1.merge(steps[1]);
    let a3 = a2.merge(steps[2]);
    let e = &v["expected"];
    for (name, a) in [("a2", &a2), ("a3", &a3)] {
        assert_eq!(
            exact(rows_json(a)),
            exact(e[name].clone()),
            "history/archives {name}"
        );
        assert_eq!(
            paused_json(a),
            e[&format!("{name}_paused")],
            "history/archives {name} paused"
        );
    }
    for c in v["retention"].as_array().unwrap() {
        let mut a = Archive {
            step: c["step_s"].as_u64().unwrap(),
            rows: c["ms"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| (m.as_u64().unwrap(), Row::of(&[Some(1.0)])))
                .collect(),
            paused: vec![],
        };
        a.retain(c["now"].as_u64().unwrap(), c["rows"].as_u64().unwrap());
        let kept: Vec<u64> = a.rows.iter().map(|r| r.0).collect();
        assert_eq!(json!(kept), c["expected"], "history/archives {}", c["name"]);
    }
}

fn limits_archives() -> Vec<config::defaults::ArchiveDefaults> {
    config::defaults::get().history.archives
}

fn range(name: &str) -> Range {
    match name {
        "1m" => Range::Min1,
        "15m" => Range::Min15,
        "1h" => Range::Hour,
        "24h" => Range::Day,
        "7d" => Range::Week,
        "all" => Range::All,
        _ => panic!("unknown range {name}"),
    }
}

#[test]
fn history_ranges() {
    let steps: Vec<u64> = limits_archives().iter().map(|a| a.step_s).collect();
    for r in load("vectors/history/ranges.json")["ranges"]
        .as_array()
        .unwrap()
    {
        let name = r["range"].as_str().unwrap();
        let got = range(name);
        let source = match got.source() {
            Source::Raw => "raw",
            Source::A1 => "a1",
            Source::A2 => "a2",
            Source::A3 => "a3",
        };
        assert_eq!(
            json!({"range": name, "source": source, "span_s": got.span_s(), "width_s": got.width_s()}),
            *r,
            "history/ranges {name}"
        );
        // A range drawn from an Archive has its step as point width.
        if let Some(i) = ["a1", "a2", "a3"].iter().position(|s| *s == source) {
            assert_eq!(got.width_s(), steps[i], "history/ranges {name} step");
        }
    }
}

fn point_json(p: &GraphPoint) -> Value {
    json!({
        "at": p.at,
        "loss": p.loss,
        "provisional": p.provisional,
        "state": match p.state { State::Value => "value", State::Lost => "lost", State::Paused => "paused" },
        "value": p.value.map(bits_json),
    })
}

fn graph_case(file: &str, v: &Value, c: &Value) -> Vec<GraphPoint> {
    let steps: Vec<u64> = limits_archives().iter().map(|a| a.step_s).collect();
    let a1 = archive(steps[0], &c["a1"], &c["a1_paused"]);
    let a2 = archive(steps[1], &c["a2"], &c["a2_paused"]);
    let a3 = archive(steps[2], &c["a3"], &c["a3_paused"]);
    let raw = if c["results"].is_array() {
        results(&c["results"])
    } else if c["raw"].is_array() {
        results(&c["raw"])
    } else {
        vec![]
    };
    let ps = pauses(&c["pauses"]);
    let src = Sources {
        raw: &raw,
        pauses: &ps,
        archives: [&a1, &a2, &a3],
        g: history::archive::grace_s(v["max_timeout_ms"].as_u64().unwrap()),
    };
    let agg = model::Aggregate::ALL
        .into_iter()
        .find(|a| a.label() == v["aggregate"].as_str().unwrap())
        .unwrap_or_else(|| panic!("{file}: aggregate"));
    let r = c["range"].as_str().map_or(Range::All, range);
    history::range::graph(r, c["now"].as_u64().unwrap(), &src, agg)
}

fn assert_points(got: &[GraphPoint], want: &Value, what: &str) {
    let want = want.as_array().unwrap();
    assert_eq!(got.len(), want.len(), "{what}: point count");
    for (g, w) in got.iter().zip(want) {
        let mut gj = point_json(g);
        let (gl, wl) = (gj["loss"].take(), &w["loss"]);
        let mut wj = w.clone();
        wj["loss"] = Value::Null;
        assert_eq!(exact(gj), exact(wj), "{what}: point at {}", g.at);
        match (gl.as_f64(), wl.as_f64()) {
            (Some(a), Some(b)) => assert!((a - b).abs() < 1e-9, "{what}: loss at {}", g.at),
            (a, b) => assert_eq!(a, b, "{what}: loss at {}", g.at),
        }
    }
}

#[test]
fn history_graphs() {
    for (file, v) in vectors("history") {
        if !file.starts_with("graph-") || file == "graph-all.json" {
            continue;
        }
        for c in v["cases"].as_array().unwrap() {
            let got = graph_case(&file, &v, c);
            assert_points(
                &got,
                &c["expected"],
                &format!("history/{file} {}", c["name"]),
            );
        }
    }
}

#[test]
fn history_graph_all() {
    let v = load("vectors/history/graph-all.json");
    let got = graph_case("graph-all.json", &v, &v);
    let oldest = v["a3"][0]["m"].as_u64().unwrap();
    let k = history::range::all_k(oldest, v["now"].as_u64().unwrap());
    assert_eq!(json!(k), v["expected"]["k"], "history/graph-all k");
    assert_points(&got, &v["expected"]["points"], "history/graph-all");
}

#[test]
fn rtt_whole_ms() {
    for c in load("vectors/rtt/whole-ms.json")["cases"]
        .as_array()
        .unwrap()
    {
        let d = std::time::Duration::from_secs_f64(c["measured_ms"].as_f64().unwrap() / 1000.0);
        assert_eq!(
            probe::rtt_ms(d) as f64,
            c["rtt_ms"].as_f64().unwrap(),
            "{c}"
        );
    }
}
