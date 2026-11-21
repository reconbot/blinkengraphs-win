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
        "vectors/probe-stats",
        "vectors/hop-trace",
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
    write(out, "vectors/meters/latency-scale.json", &latency_scale());
    for (name, v) in probe_stats() {
        write(out, &format!("vectors/probe-stats/{name}.json"), &v);
    }
    for (name, v) in hop_traces() {
        write(out, &format!("vectors/hop-trace/{name}.json"), &v);
    }
    write(out, "defaults.json", &defaults());
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
            "probes": c.probes.iter().map(|(p, st)| json!({"probe": p, "stat": stat_json(st)})).collect::<Vec<_>>(),
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
    let mut unmeasured = with(&[
        (1, st(1.0, 0.0, 12.0)),
        (2, st(1.0, 0.0, 13.0)),
        (3, st(1.0, 0.0, 14.0)),
    ]);
    unmeasured
        .probes
        .retain(|(p, _)| p.target != Target::Gateway);
    cases.push(("gateway-unmeasured", unmeasured));
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

const RAM_MINUTES: u32 = 15;

fn icmp(addr: &str) -> Probe {
    Probe {
        protocol: Protocol::Icmp,
        target: Target::Addr(addr.parse().unwrap()),
    }
}

/// Two decimals, via f64 so JSON carries no f32 noise.
fn r2(v: f32) -> f64 {
    (v as f64 * 100.0).round() / 100.0
}

fn stat_json(st: &ProbeStat) -> Value {
    json!({
        "loss": r2(st.loss),
        "rtt_ms": st.rtt_ms.map(r2),
        "baseline_ms": st.baseline_ms.map(r2),
        "jitter_ms": st.jitter_ms.map(r2),
    })
}

/// (seconds before the newest, RTT or loss)
type Results = Vec<(u64, Option<f32>)>;

fn probe_stats() -> Vec<(&'static str, Value)> {
    let steady: Vec<(u64, Option<f32>)> = (0..40)
        .map(|i| (39 - i, Some(10.0 + (i * 7 % 5) as f32)))
        .collect();
    let lossy: Vec<_> = (0..30)
        .map(|i| (29 - i, (i % 5 != 0).then_some(20.0 + (i % 3) as f32)))
        .collect();
    let all_lost: Vec<_> = (0..10).map(|i| (9 - i, None)).collect();
    let mut congested: Vec<_> = (0..300).map(|i| (329 - i, Some(10.0))).collect();
    congested.extend((0..30).map(|i| (29 - i, Some(80.0 + i as f32))));
    let mut aged_out: Vec<_> = (0..60).map(|i| (1300 - i, Some(5.0))).collect();
    aged_out.extend((0..30).map(|i| (29 - i, Some(40.0))));
    let gap = vec![
        (100, Some(9.0)),
        (31, Some(50.0)),
        (29, Some(12.0)),
        (0, Some(14.0)),
    ];
    let cases: Vec<(&str, &str, Results)> = vec![
        (
            "steady",
            "Only the last 30 s count towards loss, RTT and jitter",
            steady,
        ),
        ("loss", "Every fifth Probe lost", lossy),
        ("all-lost", "No replies: no RTT, no baseline", all_lost),
        (
            "congested",
            "Baseline comes from the whole RAM history, so RTT rises above it",
            congested,
        ),
        (
            "aged-out",
            "Results older than the RAM history are gone, baseline included",
            aged_out,
        ),
        (
            "gap",
            "The window is measured back from the newest result, not by count",
            gap,
        ),
        ("empty", "No results", vec![]),
    ];
    cases
        .into_iter()
        .map(|(name, description, results)| {
            let probe = icmp("1.1.1.1");
            let mut h = history::History::open(None, RAM_MINUTES, 7);
            for (ago, rtt) in &results {
                h.record_probe(&model::ProbeResult {
                    probe: probe.clone(),
                    at: NOW - ago,
                    rtt_ms: *rtt,
                });
            }
            let st = stat_json(&h.stat(&probe, history::RECENT_SECS));
            let results: Vec<Value> = results
                .iter()
                .map(|(ago, rtt)| json!({"at": NOW - ago, "rtt_ms": rtt}))
                .collect();
            (
                name,
                json!({
                    "name": name,
                    "description": description,
                    "window_s": history::RECENT_SECS,
                    "ram_s": RAM_MINUTES * 60,
                    "results": results,
                    "expected": st,
                }),
            )
        })
        .collect()
}

type Tick = Vec<(u8, Option<&'static str>, Option<f32>, bool)>;

fn hop_traces() -> Vec<(&'static str, Value)> {
    let a = Some("192.168.1.1");
    let b = Some("10.0.0.1");
    let t = Some("1.1.1.1");
    let full = |r2: Option<f32>| -> Tick {
        vec![
            (1, a, Some(1.0), false),
            (2, b.filter(|_| r2.is_some()), r2, false),
            (3, t, Some(12.0), true),
        ]
    };
    let cases: Vec<(&str, &str, Vec<Tick>)> = vec![
        (
            "basic",
            "Three sweeps; hop 2 loses one reply and keeps its last known address",
            vec![full(Some(5.0)), full(None), full(Some(7.0))],
        ),
        (
            "path-shortens",
            "When the target answers at a lower TTL, rows beyond it are dropped",
            vec![
                full(Some(5.0)),
                vec![(1, a, Some(1.0), false), (2, t, Some(9.0), true)],
            ],
        ),
        (
            "window",
            "Each hop keeps only its last HOP_WINDOW results",
            (0..history::HOP_WINDOW as u32 + 5)
                .map(|i| vec![(1, t, (i % 2 == 0).then_some(1.0 + i as f32), true)])
                .collect(),
        ),
    ];
    cases
        .into_iter()
        .map(|(name, description, ticks)| {
            let trace = icmp("1.1.1.1");
            let mut h = history::History::open(None, RAM_MINUTES, 7);
            for (i, tick) in ticks.iter().enumerate() {
                for (ttl, addr, rtt, reached) in tick {
                    h.record_hop(&model::HopResult {
                        trace: trace.clone(),
                        at: NOW + i as u64,
                        ttl: *ttl,
                        addr: addr.map(|s| s.parse().unwrap()),
                        rtt_ms: *rtt,
                        reached: *reached,
                    });
                }
            }
            let ticks: Vec<Value> = ticks
                .iter()
                .map(|tick| {
                    tick.iter()
                        .map(|(ttl, addr, rtt, reached)| {
                            json!({"ttl": ttl, "addr": addr, "rtt_ms": rtt, "reached": reached})
                        })
                        .collect()
                })
                .collect();
            let hops: Vec<Value> = h
                .hops(&trace)
                .iter()
                .map(|x| {
                    json!({
                        "ttl": x.ttl, "addr": x.addr, "sent": x.sent, "loss": r2(x.loss),
                        "last_ms": x.last_ms.map(r2), "best_ms": x.best_ms.map(r2),
                        "avg_ms": x.avg_ms.map(r2), "worst_ms": x.worst_ms.map(r2),
                    })
                })
                .collect();
            (
                name,
                json!({
                    "name": name,
                    "description": description,
                    "hop_window": history::HOP_WINDOW,
                    "ticks": ticks,
                    "expected": hops,
                }),
            )
        })
        .collect()
}

fn latency_scale() -> Value {
    let points: Vec<Value> = [None, Some(5.0), Some(25.0), Some(40.0), Some(120.0)]
        .into_iter()
        .map(|baseline| {
            let scale = meters::latency_scale(baseline);
            let rtts = [10.0f32, 60.0, 150.0, 1000.0];
            json!({
                "baseline_ms": baseline,
                "scale_ms": scale,
                "normalized": rtts.iter().map(|r| json!({"rtt_ms": r, "value": r2((r / scale).min(1.0))})).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "description": "The Latency Meter's full height is max(4 × baseline, 100 ms); a missing baseline counts as 20 ms. A reply draws min(rtt / scale, 1); a loss fills the column in the loss color.",
        "points": points,
    })
}

fn defaults() -> Value {
    let c = config::Config::default();
    let g = &c.groups[0];
    json!({
        "description": "Defaults of the Windows reference implementation. Probe intervals are in ms; a Probe's timeout is min(interval, max_timeout_ms).",
        "primary": c.primary,
        "probe_group": {
            "name": g.name,
            "latency_ms": g.latency_ms,
            "dns_ms": g.dns_ms,
            "trace_ms": g.trace_ms,
            "targets": g.targets,
        },
        "max_timeout_ms": probe::MAX_TIMEOUT.as_millis() as u64,
        "dns_query_name": probe::QUERY_NAME,
        "hop_trace": {
            "max_ttl": probe::MAX_TTL,
            "full_sweep_every_s": probe::FULL_SWEEP_EVERY.as_secs(),
            "hop_window": history::HOP_WINDOW,
        },
        "history": {
            "ram_minutes": c.history.ram_minutes,
            "rollup_days": c.history.rollup_days,
            "recent_window_s": history::RECENT_SECS,
        },
        "system_interval_ms": c.system_interval_ms,
    })
}
