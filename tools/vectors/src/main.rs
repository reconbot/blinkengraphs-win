//! Usage: vectors <out_dir>
//! Writes the spec repo's test vectors, design tokens and fixture scenarios from this reference implementation.
//! `defaults.json` is not generated: the spec owns it, and `config::defaults` embeds it.

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
        "vectors/sketch",
        "vectors/defaults",
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
    write(out, "vectors/defaults/merge.json", &defaults_merge());
    for (name, v) in sketch_vectors() {
        write(out, &format!("vectors/sketch/{name}.json"), &v);
    }
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
    path_up: bool,
    via_vpn: bool,
}

fn verdict_vector(name: &str, c: &Case) -> Value {
    let findings = verdict::diagnose(&verdict::Input {
        sample: c.sample.as_ref(),
        probes: &c.probes,
        upload_peak_bps: c.upload_peak_bps,
        path_up: c.path_up,
        via_vpn: c.via_vpn,
    });
    let sample = c.sample.as_ref().map(|s| {
        Metric::ALL
            .iter()
            .map(|m| (format!("{m:?}"), json!(s.get(*m))))
            .collect::<serde_json::Map<_, _>>()
    });
    let mut input = json!({
        "sample": sample,
        "probes": c.probes.iter().map(|(p, st)| json!({"probe": p, "stat": stat_json(st)})).collect::<Vec<_>>(),
        "upload_peak_bps": c.upload_peak_bps,
    });
    // Absent means true, so the vectors written before the Offline rule stay valid.
    if !c.path_up {
        input["path_up"] = json!(false);
    }
    if c.via_vpn {
        input["via_vpn"] = json!(true);
    }
    json!({"name": name, "input": input, "expected": findings})
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
                    path_up: true,
                    via_vpn: false,
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
            path_up: true,
            via_vpn: false,
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
            path_up: true,
            via_vpn: false,
        },
    ));
    let dual = |v4_loss: f32, v6_loss: f32| {
        let mut ps: Vec<_> = healthy()
            .into_iter()
            .map(|(p, s)| {
                let v4 = matches!(p.target, Target::Addr(a) if a.is_ipv4());
                if v4 && p.protocol != Protocol::Dns && v4_loss > 0.0 {
                    (p, st(v4_loss, 0.0, s.baseline_ms.unwrap()))
                } else {
                    (p, s)
                }
            })
            .collect();
        ps.push((
            p(Protocol::Icmp, "2606:4700:4700::1111"),
            st(v6_loss, 13.0, 13.0),
        ));
        ps.push((
            p(Protocol::Tcp(443), "2606:4700:4700::1111"),
            st(v6_loss, 14.0, 14.0),
        ));
        ps.push((p(Protocol::Icmp, "2620:fe::fe"), st(v6_loss, 15.0, 15.0)));
        Case {
            sample: None,
            probes: ps,
            upload_peak_bps: 5e6,
            path_up: true,
            via_vpn: false,
        }
    };
    cases.push(("dual-stack-clear", dual(0.0, 0.0)));
    cases.push(("ipv6-broken", dual(0.0, 1.0)));
    cases.push(("ipv4-broken", dual(1.0, 0.0)));
    // Through a VPN: what would blame the ISP or ping filtering names the tunnel.
    let vpn = |mut c: Case| {
        c.via_vpn = true;
        c
    };
    cases.push(("vpn-ipv6-broken", vpn(dual(0.0, 1.0))));
    let internet_loss = |loss: f32, with_gateway: bool| {
        let probes = healthy()
            .into_iter()
            .filter(|(p, _)| with_gateway || p.target != Target::Gateway)
            .map(|(p, s)| {
                let internet = !matches!(p.target, Target::Gateway | Target::SystemResolver)
                    && p.protocol != Protocol::Dns;
                if internet {
                    (p, st(loss, 30.0, s.baseline_ms.unwrap()))
                } else {
                    (p, s)
                }
            })
            .collect();
        Case {
            sample: None,
            probes,
            upload_peak_bps: 5e6,
            path_up: true,
            via_vpn: true,
        }
    };
    cases.push(("vpn-isp-loss", internet_loss(0.25, true)));
    cases.push(("vpn-unreachable", internet_loss(1.0, false)));
    let icmp_filtered = {
        let mut c = internet_loss(0.0, true);
        for (p, s) in c.probes.iter_mut() {
            if p.protocol == Protocol::Icmp && p.target != Target::Gateway {
                *s = st(1.0, 0.0, s.baseline_ms.unwrap());
            }
        }
        c
    };
    cases.push(("vpn-icmp-filtered", icmp_filtered));
    cases.push((
        "offline",
        Case {
            sample: None,
            probes: healthy()
                .into_iter()
                .map(|(p, _)| (p, st(1.0, 0.0, 10.0)))
                .collect(),
            upload_peak_bps: 5e6,
            path_up: false,
            via_vpn: false,
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

fn defaults_merge() -> Value {
    let cases = [
        (
            "objects merge key by key",
            json!({"history": {"ram_minutes": 15, "rollup_days": 7}, "system_interval_ms": 1000}),
            json!({"history": {"rollup_days": 30}}),
        ),
        (
            "arrays and scalars replace whole",
            json!({"targets": [{"target": "1.1.1.1"}, {"target": "9.9.9.9"}], "max_timeout_ms": 1000}),
            json!({"targets": [{"target": "1.1.1.1"}], "max_timeout_ms": 500}),
        ),
        (
            "null removes a key; new keys are added",
            json!({"system_interval_ms": 1000, "dns_query_name": "cloudflare.com"}),
            json!({"system_interval_ms": null, "battery_saver": true}),
        ),
        (
            "an empty override changes nothing",
            json!({"probe_group": {"name": "Default", "latency_ms": 1000}}),
            json!({}),
        ),
    ];
    json!({
        "description": "Merging a platform's defaults.override.json over the spec's defaults.json (rules/defaults.md).",
        "cases": cases.into_iter().map(|(name, base, over)| json!({
            "name": name,
            "base": base,
            "override": over,
            "expected": config::defaults::merge(base.clone(), over.clone()),
        })).collect::<Vec<_>>(),
    })
}

fn bits(v: f64) -> Value {
    json!({"value": v, "bits": format!("{:016x}", v.to_bits())})
}

fn minute_json(m: &history::Minute) -> Value {
    json!({
        "n": m.n, "lost": m.lost,
        "sum": (m.n > 0).then(|| bits(m.sum)),
        "min": (m.n > 0).then(|| bits(m.min)),
        "max": (m.n > 0).then(|| bits(m.max)),
        "sk": m.sketch.encode().iter().map(|b| format!("{b:02x}")).collect::<String>(),
    })
}

/// Values a minute might hold: RTTs with jitter and a lost Probe, then a zero and a huge rate.
fn sketch_minutes() -> Vec<Vec<Option<f32>>> {
    let rtt = |base: f32| -> Vec<Option<f32>> {
        (0..60)
            .map(|i| (i % 17 != 0).then_some(base + (i * 37 % 23) as f32 * 0.7))
            .collect()
    };
    vec![
        rtt(12.0),
        rtt(18.5),
        rtt(95.0),
        vec![Some(0.0), Some(0.25), Some(3.5e9), None],
    ]
}

fn sketch_vectors() -> Vec<(&'static str, Value)> {
    use history::{Minute, Sketch};
    use model::Aggregate;
    let below = f64::from_bits((1.0f64 / 1024.0).to_bits() - 1);
    let below_one = f64::from_bits(1f64.to_bits() - 1);
    let below_33 = f64::from_bits((33.0f64 / 32.0).to_bits() - 1);
    let index: Vec<Value> = [
        0.0,
        -1.0,
        below,
        1.0 / 1024.0,
        below_one,
        1.0,
        below_33,
        33.0 / 32.0,
        12.3,
        1e10,
        2f64.powi(40),
        1e30,
    ]
    .into_iter()
    .map(|v| {
        let i = history::sketch_index(v);
        json!({"value": bits(v), "index": i, "rep": i.map(|i| bits(history::sketch_rep(i)))})
    })
    .collect();
    let minutes: Vec<Minute> = sketch_minutes().iter().map(|vs| Minute::of(vs)).collect();
    let merged = |order: &[usize]| {
        let mut acc = Minute::default();
        for i in order {
            acc.merge(&minutes[*i]);
        }
        acc
    };
    let forward = merged(&[0, 1, 2, 3]);
    let aggregates = |m: &Minute| -> Value {
        Aggregate::ALL
            .iter()
            .map(|a| {
                (
                    a.label().to_string(),
                    m.value(*a).map(bits).unwrap_or(Value::Null),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    let reversed = {
        let mut s = Sketch::default();
        for i in [3, 2, 1, 0] {
            s.merge(&minutes[i].sketch);
        }
        s
    };
    assert_eq!(reversed, forward.sketch, "merge order must not matter");
    vec![
        (
            "index",
            json!({
                "description": "Bucket index (null = zero bucket) and representative value for chosen inputs. f64 values carry their exact bits.",
                "cases": index,
            }),
        ),
        (
            "minutes",
            json!({
                "description": "Per-minute rollups from raw values in arrival order (null = lost); sk is the hex of the encoded sketch.",
                "cases": sketch_minutes().iter().zip(&minutes).map(|(vs, m)| json!({"values": vs, "minute": minute_json(m)})).collect::<Vec<_>>(),
            }),
        ),
        (
            "merge",
            json!({
                "description": "Merging the minutes of the minutes vector in ascending order, and the aggregates a graph bucket shows. Merging sketches in any order gives the same sk.",
                "merged": minute_json(&forward),
                "loss": r2(forward.loss()),
                "aggregates": aggregates(&forward),
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
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
}
