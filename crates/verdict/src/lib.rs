use model::{Metric, Probe, ProbeStat, Protocol, Sample, Target};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum FaultDomain {
    Clear,
    LocalLoad,
    Lan,
    Isp,
    Dns,
    Remote,
    IcmpFiltering,
    SelfSaturation,
    /// No usable network path at all: airplane mode, radios off, cable out.
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum Severity {
    Ok,
    Warn,
    Crit,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Finding {
    pub domain: FaultDomain,
    pub severity: Severity,
    pub headline: String,
}

pub struct Input<'a> {
    pub sample: Option<&'a Sample>,
    /// Recent window per Probe (≈30 s).
    pub probes: &'a [(Probe, ProbeStat)],
    /// Highest upload rate seen in RAM History; stands in for the unknown link capacity.
    pub upload_peak_bps: f64,
    /// Whether the OS reports any usable network path. Without one, Probes are paused and
    /// their stats only say the device is offline.
    pub path_up: bool,
}

const LOSS_WARN: f32 = 0.05;
const LOSS_DOWN: f32 = 0.5;
const INFLATED: f32 = 3.0;

/// Findings for the current state of `h`: recent Probe stats, last Sample, upload peak over RAM.
pub fn from_history(h: &history::History, path_up: bool) -> Vec<Finding> {
    let probes: Vec<(Probe, ProbeStat)> = h
        .probes()
        .into_iter()
        .map(|p| {
            let st = h.stat(&p, history::RECENT_SECS);
            (p, st)
        })
        .collect();
    let peak = h
        .recent(&history::Key::Metric(Metric::NetUp), usize::MAX)
        .into_iter()
        .flatten()
        .fold(0.0f32, f32::max) as f64;
    diagnose(&Input {
        sample: h.last_sample(),
        probes: &probes,
        upload_peak_bps: peak,
        path_up,
    })
}

/// Most severe first; never empty.
pub fn diagnose(input: &Input) -> Vec<Finding> {
    let mut out = if input.path_up {
        network(input)
    } else {
        vec![Finding {
            domain: FaultDomain::Offline,
            severity: Severity::Crit,
            headline: "No network connection".into(),
        }]
    };
    out.extend(local(input));
    if out.is_empty() {
        out.push(Finding {
            domain: FaultDomain::Clear,
            severity: Severity::Ok,
            headline: "All clear".into(),
        });
    }
    out.sort_by_key(|f| std::cmp::Reverse(f.severity));
    out
}

fn network(input: &Input) -> Vec<Finding> {
    let reach = |p: &&(Probe, ProbeStat)| {
        p.0.protocol != Protocol::Dns && p.1.baseline_ms.is_some() || p.1.loss > 0.0
    };
    let gateway: Vec<_> = input
        .probes
        .iter()
        .filter(|p| p.0.target == Target::Gateway)
        .filter(reach)
        .collect();
    let internet: Vec<_> = input
        .probes
        .iter()
        .filter(|p| {
            !matches!(p.0.target, Target::Gateway | Target::SystemResolver)
                && p.0.protocol != Protocol::Dns
        })
        .filter(reach)
        .collect();
    let dns: Vec<_> = input
        .probes
        .iter()
        .filter(|p| p.0.protocol == Protocol::Dns)
        .collect();
    let mut out = Vec::new();

    let worst = |ps: &[&(Probe, ProbeStat)]| ps.iter().map(|p| p.1.loss).fold(0.0f32, f32::max);
    let best = |ps: &[&(Probe, ProbeStat)]| ps.iter().map(|p| p.1.loss).fold(1.0f32, f32::min);
    let gw_loss = worst(&gateway);
    let net_best = best(&internet);

    if !gateway.is_empty() && gw_loss >= LOSS_DOWN && net_best >= LOSS_DOWN {
        out.push(crit(
            FaultDomain::Lan,
            "Gateway unreachable: Wi-Fi or LAN problem".into(),
        ));
        return out;
    }
    if gw_loss >= LOSS_WARN {
        out.push(warn(
            FaultDomain::Lan,
            format!("Local network unstable: gateway losing {}", pct(gw_loss)),
        ));
    }

    if !internet.is_empty() {
        let icmp_lost = internet
            .iter()
            .filter(|p| p.0.protocol == Protocol::Icmp)
            .all(|p| p.1.loss >= LOSS_DOWN);
        let tcp_ok = internet
            .iter()
            .any(|p| matches!(p.0.protocol, Protocol::Tcp(_)) && p.1.loss < LOSS_WARN);
        let any_icmp = internet.iter().any(|p| p.0.protocol == Protocol::Icmp);
        // Without a measured Gateway (none found, or the OS denied local network access) the LAN can't be cleared.
        let gw = if gateway.is_empty() {
            ""
        } else {
            "Gateway OK, "
        };
        if net_best >= LOSS_DOWN {
            let headline = if gateway.is_empty() {
                "Internet unreachable, gateway not measured: ISP or WAN problem".into()
            } else {
                "Gateway OK, internet unreachable: ISP or WAN problem".into()
            };
            out.push(crit(FaultDomain::Isp, headline));
        } else if any_icmp && icmp_lost && tcp_ok {
            out.push(warn(
                FaultDomain::IcmpFiltering,
                "ICMP blocked; TCP gets through: something filters ping".into(),
            ));
        } else if let Some(bad) = internet
            .iter()
            .filter(|p| p.1.loss >= LOSS_WARN)
            .max_by(|a, b| a.1.loss.total_cmp(&b.1.loss))
        {
            let all_bad = internet.iter().all(|p| p.1.loss >= LOSS_WARN);
            if all_bad {
                let sev = if bad.1.loss >= 0.2 {
                    Severity::Crit
                } else {
                    Severity::Warn
                };
                out.push(Finding {
                    domain: FaultDomain::Isp,
                    severity: sev,
                    headline: format!(
                        "{gw}{} losing {}: ISP problem",
                        bad.0.target,
                        pct(bad.1.loss)
                    ),
                });
            } else if let Some(good) = internet.iter().find(|p| p.1.loss < LOSS_WARN) {
                out.push(warn(
                    FaultDomain::Remote,
                    format!(
                        "{} losing {}, {} fine: problem is on their side",
                        bad.0.target,
                        pct(bad.1.loss),
                        good.0.target
                    ),
                ));
            }
        } else if let Some(s) = self_saturation(input, &internet) {
            out.push(s);
        }
    }

    let dns_bad =
        |p: &&(Probe, ProbeStat)| p.1.loss >= 0.3 || p.1.rtt_ms.is_some_and(|r| r > 250.0);
    let resolver = dns.iter().find(|p| p.0.target == Target::SystemResolver);
    let public_ok = dns
        .iter()
        .any(|p| p.0.target != Target::SystemResolver && !dns_bad(p));
    if let Some(r) = resolver.filter(|r| dns_bad(r)) {
        let what = match r.1.rtt_ms {
            Some(ms) if r.1.loss < 0.3 => format!("slow ({ms:.0} ms)"),
            _ => format!("losing {}", pct(r.1.loss)),
        };
        let suffix = if public_ok {
            "; 1.1.1.1 answers fine"
        } else {
            ""
        };
        out.push(warn(
            FaultDomain::Dns,
            format!("Your DNS server is {what}{suffix}"),
        ));
    }
    out
}

fn self_saturation(input: &Input, internet: &[&(Probe, ProbeStat)]) -> Option<Finding> {
    let up = input.sample?.get(Metric::NetUp);
    let inflated = internet
        .iter()
        .filter_map(|p| Some((p.1.rtt_ms?, p.1.baseline_ms?)))
        .find(|(r, b)| *r > b * INFLATED && *r - *b > 30.0)?;
    (up > 1e6 && up >= input.upload_peak_bps * 0.8).then(|| {
        warn(
            FaultDomain::SelfSaturation,
            format!(
                "Upload saturated ({}/s): {:.0} ms vs {:.0} ms normal",
                bytes(up),
                inflated.0,
                inflated.1
            ),
        )
    })
}

fn local(input: &Input) -> Vec<Finding> {
    let Some(s) = input.sample else { return vec![] };
    let mut out = Vec::new();
    let cpu = s.get(Metric::CpuTotal);
    if cpu >= 90.0 {
        out.push(crit(
            FaultDomain::LocalLoad,
            format!("CPU pinned at {cpu:.0}%"),
        ));
    }
    let commit = s.get(Metric::MemCommit);
    let faults = s.get(Metric::MemHardFaults);
    if commit >= 90.0 {
        out.push(crit(
            FaultDomain::LocalLoad,
            format!("Memory nearly exhausted: commit {commit:.0}%"),
        ));
    } else if faults >= 1000.0 {
        out.push(warn(
            FaultDomain::LocalLoad,
            format!("Paging heavily: {faults:.0} hard faults/s"),
        ));
    }
    let disk = s.get(Metric::DiskActive);
    let resp = s.get(Metric::DiskResponse);
    if disk >= 90.0 && resp >= 50.0 {
        out.push(warn(
            FaultDomain::LocalLoad,
            format!("Disk saturated: {disk:.0}% busy, {resp:.0} ms response"),
        ));
    }
    out
}

fn crit(domain: FaultDomain, headline: String) -> Finding {
    Finding {
        domain,
        severity: Severity::Crit,
        headline,
    }
}

fn warn(domain: FaultDomain, headline: String) -> Finding {
    Finding {
        domain,
        severity: Severity::Warn,
        headline,
    }
}

fn pct(f: f32) -> String {
    format!("{:.0}%", f * 100.0)
}

pub fn bytes(b: f64) -> String {
    match b {
        b if b >= 1e9 => format!("{:.1} GB", b / 1e9),
        b if b >= 1e6 => format!("{:.1} MB", b / 1e6),
        b if b >= 1e3 => format!("{:.0} KB", b / 1e3),
        b => format!("{b:.0} B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(protocol: Protocol, target: &str) -> Probe {
        let target = match target {
            "gw" => Target::Gateway,
            "dns" => Target::SystemResolver,
            t => Target::Addr(t.parse().unwrap()),
        };
        Probe { target, protocol }
    }

    fn st(loss: f32, rtt: f32, baseline: f32) -> ProbeStat {
        ProbeStat {
            loss,
            rtt_ms: (loss < 1.0).then_some(rtt),
            baseline_ms: Some(baseline),
            jitter_ms: None,
        }
    }

    fn healthy() -> Vec<(Probe, ProbeStat)> {
        vec![
            (p(Protocol::Icmp, "gw"), st(0.0, 2.0, 2.0)),
            (p(Protocol::Icmp, "1.1.1.1"), st(0.0, 12.0, 12.0)),
            (p(Protocol::Tcp(443), "1.1.1.1"), st(0.0, 13.0, 13.0)),
            (p(Protocol::Icmp, "8.8.8.8"), st(0.0, 14.0, 14.0)),
            (p(Protocol::Dns, "1.1.1.1"), st(0.0, 12.0, 12.0)),
            (p(Protocol::Dns, "dns"), st(0.0, 20.0, 20.0)),
        ]
    }

    fn run(probes: &[(Probe, ProbeStat)], sample: Option<&Sample>) -> Vec<Finding> {
        diagnose(&Input {
            sample,
            probes,
            upload_peak_bps: 5e6,
            path_up: true,
        })
    }

    fn set(probes: &mut [(Probe, ProbeStat)], i: usize, s: ProbeStat) {
        probes[i].1 = s;
    }

    #[test]
    fn healthy_is_clear() {
        assert_eq!(run(&healthy(), None)[0].domain, FaultDomain::Clear);
    }

    #[test]
    fn wifi_drop() {
        let ps: Vec<_> = healthy()
            .into_iter()
            .map(|(p, _)| (p, st(1.0, 0.0, 10.0)))
            .collect();
        let f = run(&ps, None);
        assert_eq!(
            (f[0].domain, f[0].severity),
            (FaultDomain::Lan, Severity::Crit)
        );
    }

    #[test]
    fn isp_loss() {
        let mut ps = healthy();
        for i in 1..=3 {
            set(&mut ps, i, st(0.12, 40.0, 12.0));
        }
        let f = run(&ps, None);
        assert_eq!(f[0].domain, FaultDomain::Isp);
        assert!(f[0].headline.contains("12%"), "{}", f[0].headline);
    }

    #[test]
    fn unmeasured_gateway_is_not_called_ok() {
        let mut ps: Vec<_> = healthy().into_iter().skip(1).collect();
        for i in 0..=2 {
            set(&mut ps, i, st(1.0, 0.0, 12.0));
        }
        let f = run(&ps, None);
        assert_eq!(f[0].domain, FaultDomain::Isp);
        assert!(!f[0].headline.contains("Gateway OK"), "{}", f[0].headline);
        for i in 0..=2 {
            set(&mut ps, i, st(0.12, 40.0, 12.0));
        }
        let f = run(&ps, None);
        assert!(!f[0].headline.contains("Gateway OK"), "{}", f[0].headline);
    }

    #[test]
    fn icmp_filtered() {
        let mut ps = healthy();
        set(&mut ps, 1, st(1.0, 0.0, 12.0));
        set(&mut ps, 3, st(1.0, 0.0, 14.0));
        assert_eq!(run(&ps, None)[0].domain, FaultDomain::IcmpFiltering);
    }

    #[test]
    fn one_remote_down() {
        let mut ps = healthy();
        set(&mut ps, 3, st(0.4, 30.0, 14.0));
        let f = run(&ps, None);
        assert_eq!(f[0].domain, FaultDomain::Remote);
        assert!(f[0].headline.starts_with("8.8.8.8"));
    }

    #[test]
    fn slow_dns() {
        let mut ps = healthy();
        set(&mut ps, 5, st(0.0, 600.0, 20.0));
        let f = run(&ps, None);
        assert_eq!(f[0].domain, FaultDomain::Dns);
        assert!(f[0].headline.contains("1.1.1.1 answers fine"));
    }

    #[test]
    fn upload_saturation() {
        let mut ps = healthy();
        set(&mut ps, 1, st(0.0, 180.0, 12.0));
        let mut s = Sample::default();
        s.set(Metric::NetUp, 5e6);
        let f = run(&ps, Some(&s));
        assert_eq!(f[0].domain, FaultDomain::SelfSaturation, "{f:?}");
    }

    #[test]
    fn cpu_pinned_outranks_dns_warning() {
        let mut ps = healthy();
        set(&mut ps, 5, st(0.0, 600.0, 20.0));
        let mut s = Sample::default();
        s.set(Metric::CpuTotal, 97.0);
        let f = run(&ps, Some(&s));
        assert_eq!(
            (f[0].domain, f[0].severity),
            (FaultDomain::LocalLoad, Severity::Crit)
        );
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn offline_replaces_every_network_rule() {
        let probes = [
            (p(Protocol::Icmp, "1.1.1.1"), lost()),
            (p(Protocol::Dns, "dns"), lost()),
        ];
        let got = diagnose(&Input {
            sample: None,
            probes: &probes,
            upload_peak_bps: 0.0,
            path_up: false,
        });
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].domain, FaultDomain::Offline);
        assert_eq!(got[0].severity, Severity::Crit);
        assert_eq!(got[0].headline, "No network connection");
    }

    fn lost() -> ProbeStat {
        ProbeStat {
            loss: 1.0,
            rtt_ms: None,
            baseline_ms: Some(10.0),
            jitter_ms: None,
        }
    }
}
