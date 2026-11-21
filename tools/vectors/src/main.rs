//! Usage: vectors <out_dir>
//! Writes the spec repo's test vectors, design tokens and fixture scenarios from this reference implementation.

use model::{Metric, Probe, ProbeStat, Protocol, Sample, Target};
use serde_json::{Value, json};
use std::{fs, path::Path};

const NOW: u64 = 1_800_000_000;

fn main() {
    let out = std::env::args().nth(1).expect("usage: vectors <out_dir>");
    let out = Path::new(&out);
    for d in [
        "vectors/verdict",
        "vectors/meters",
        "tokens",
        "fixtures/scenarios",
    ] {
        fs::create_dir_all(out.join(d)).unwrap();
    }
    for (name, input) in verdict_cases() {
        write(
            out,
            &format!("vectors/verdict/{name}.json"),
            &verdict_vector(name, &input),
        );
    }
    write(out, "vectors/meters/cube-root.json", &cube_root());
    write(out, "tokens/blinkengraphs.tokens.json", &tokens());
    for s in fixtures::all() {
        write(
            out,
            &format!("fixtures/scenarios/{}.json", s.name),
            &json!({
                "name": s.name, "caption": s.caption, "interval_s": 1, "normalized": true,
                "cpu": s.cpu, "memory": s.memory, "disk": s.disk, "down": s.down, "up": s.up, "rtt": s.rtt,
            }),
        );
    }
}

fn write(out: &Path, rel: &str, v: &Value) {
    fs::write(
        out.join(rel),
        serde_json::to_string_pretty(v).unwrap() + "\n",
    )
    .unwrap();
}

struct Case {
    sample: Option<Sample>,
    probes: Vec<(Probe, ProbeStat)>,
    upload_peak_bps: f64,
}

fn verdict_vector(name: &str, c: &Case) -> Value {
    let findings = verdict::diagnose(&verdict::Input {
        sample: c.sample.as_ref(),
        probes: &c.probes,
        upload_peak_bps: c.upload_peak_bps,
    });
    let sample = c.sample.as_ref().map(|s| {
        Metric::ALL
            .iter()
            .map(|m| (format!("{m:?}"), json!(s.get(*m))))
            .collect::<serde_json::Map<_, _>>()
    });
    json!({
        "name": name,
        "input": {
            "sample": sample,
            "probes": c.probes.iter().map(|(p, st)| json!({"probe": p, "stat": st})).collect::<Vec<_>>(),
            "upload_peak_bps": c.upload_peak_bps,
        },
        "expected": findings,
    })
}

fn verdict_cases() -> Vec<(&'static str, Case)> {
    let mut cases: Vec<(&'static str, Case)> = fixtures::all()
        .into_iter()
        .map(|s| {
            let mut h = history::History::open(None, 15, 7);
            fixtures::populate(&mut h, &s, NOW);
            let probes = h.probes().into_iter().map(|p| {
                let st = h.stat(&p, history::RECENT_SECS);
                (p, st)
            });
            let mut probes: Vec<_> = probes.map(|(p, st)| (p, round_stat(st))).collect();
            probes.sort_by_key(|(p, _)| p.to_string());
            let peak = h
                .recent(&history::Key::Metric(Metric::NetUp), usize::MAX)
                .into_iter()
                .flatten()
                .fold(0.0f32, f32::max) as f64;
            let sample = h.last_sample().cloned().map(|mut x| {
                for m in Metric::ALL {
                    x.set(m, (x.get(m) * 100.0).round() / 100.0);
                }
                x.per_core.clear();
                x
            });
            (
                s.name,
                Case {
                    sample,
                    probes,
                    upload_peak_bps: peak.round(),
                },
            )
        })
        .collect();

    let p = |protocol, t: &str| Probe {
        protocol,
        target: match t {
            "gw" => Target::Gateway,
            "dns" => Target::SystemResolver,
            a => Target::Addr(a.parse().unwrap()),
        },
    };
    let st = |loss: f32, rtt: f32, base: f32| ProbeStat {
        loss,
        rtt_ms: (loss < 1.0).then_some(rtt),
        baseline_ms: Some(base),
        jitter_ms: None,
    };
    let healthy = || {
        vec![
            (p(Protocol::Icmp, "gw"), st(0.0, 2.0, 2.0)),
            (p(Protocol::Icmp, "1.1.1.1"), st(0.0, 12.0, 12.0)),
            (p(Protocol::Tcp(443), "1.1.1.1"), st(0.0, 13.0, 13.0)),
            (p(Protocol::Icmp, "8.8.8.8"), st(0.0, 14.0, 14.0)),
            (p(Protocol::Dns, "1.1.1.1"), st(0.0, 12.0, 12.0)),
            (p(Protocol::Dns, "dns"), st(0.0, 20.0, 20.0)),
        ]
    };
    let with = |edits: &[(usize, ProbeStat)]| {
        let mut ps = healthy();
        for (i, s) in edits {
            ps[*i].1 = s.clone();
        }
        Case {
            sample: None,
            probes: ps,
            upload_peak_bps: 5e6,
        }
    };
    cases.push((
        "lan-down",
        Case {
            sample: None,
            probes: healthy()
                .into_iter()
                .map(|(p, _)| (p, st(1.0, 0.0, 10.0)))
                .collect(),
            upload_peak_bps: 5e6,
        },
    ));
    cases.push((
        "icmp-filtered",
        with(&[(1, st(1.0, 0.0, 12.0)), (3, st(1.0, 0.0, 14.0))]),
    ));
    cases.push(("remote-loss", with(&[(3, st(0.4, 30.0, 14.0))])));
    cases.push(("slow-dns", with(&[(5, st(0.0, 600.0, 20.0))])));
    cases.push(("all-clear", with(&[])));
    cases
}

fn round_stat(st: ProbeStat) -> ProbeStat {
    let r = |v: f32| (v * 100.0).round() / 100.0;
    ProbeStat {
        loss: r(st.loss),
        rtt_ms: st.rtt_ms.map(r),
        baseline_ms: st.baseline_ms.map(r),
        jitter_ms: st.jitter_ms.map(r),
    }
}

fn cube_root() -> Value {
    let points: Vec<Value> = [0.0, 1e3, 1e4, 1.25e5, 1e6, 8e6, 1.25e7, 2.5e7]
        .iter()
        .map(|v| json!({"value": v, "max": 1.25e7, "normalized": render::cube_root(*v, 1.25e7)}))
        .collect();
    json!({
        "description": "Network Meters scale throughput as cbrt(value / max), clamped to 1, so small and large rates both stay visible.",
        "points": points,
    })
}

fn tokens() -> Value {
    let c = |hex: &str| json!({"$type": "color", "$value": hex});
    let n = |v: f32| json!({"$type": "number", "$value": v});
    json!({
        "color": {
            "section": {
                "cpu": c("#4fc3f7"), "memory": c("#9ccc65"), "network": c("#7ab8ff"),
                "disk": c("#b39ddb"), "latency": c("#ffd54f"), "upload": c("#ff9be1")
            },
            "severity": { "ok": c("#4ade80"), "warn": c("#ffb340"), "crit": c("#ff5a5a") },
            "loss": c("#ff3b3b"),
            "surface": { "background": c("#12151c"), "card": c("#1b202a"), "card-edge": c("#2a313d"),
                         "text": c("#e6eaf0"), "text-weak": c("#8b95a5") }
        },
        "threshold": { "meter-warn": n(render::WARN), "meter-crit": n(render::CRIT) }
    })
}
