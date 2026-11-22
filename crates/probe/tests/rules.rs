//! The Prober's spec rules (`rules/probe-stats.md`, `rules/hop-trace.md`) over a scripted network.

use model::{HopResult, Probe, ProbeResult, Protocol, Target};
use probe::{Echo, Event, Limits, ProbeGroup, Prober, Transport, transport::Hop};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::mpsc::Receiver,
    time::Duration,
};

const V4: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
const V6: IpAddr = IpAddr::V6(std::net::Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
/// The target in the scripted network is this many hops away.
const HOPS: u8 = 3;

struct Net {
    v4: bool,
    v6: bool,
}

impl Transport for Net {
    fn family_up(&self, v6: bool) -> bool {
        if v6 { self.v6 } else { self.v4 }
    }
    fn lookup(&self, t: &Target) -> Option<IpAddr> {
        match t {
            Target::Addr(a) => Some(*a),
            _ => None,
        }
    }
    fn tcp(&self, _: SocketAddr, _: Duration) -> Option<Duration> {
        Some(Duration::from_millis(7))
    }
    fn udp(&self, _: SocketAddr, _: Duration) -> Option<Duration> {
        None
    }
    fn dns(&self, _: SocketAddr, _: Duration, _: &str) -> Option<Duration> {
        Some(Duration::from_millis(9))
    }
    fn echo(&self, requests: &[(IpAddr, u8)], _: Duration) -> Option<Vec<Option<Echo>>> {
        Some(
            requests
                .iter()
                .map(|&(to, ttl)| {
                    Some(Echo {
                        from: if ttl >= HOPS {
                            to
                        } else {
                            IpAddr::V4(Ipv4Addr::new(10, 0, 0, ttl))
                        },
                        rtt: Duration::from_millis(ttl as u64),
                        reached: ttl >= HOPS,
                    })
                })
                .collect(),
        )
    }
    fn tcp_trace(&self, _: Ipv4Addr, _: u16, _: u8, _: Duration) -> Option<Vec<Hop>> {
        None
    }
}

fn limits() -> Limits {
    Limits {
        max_timeout: Duration::from_secs(1),
        dns_query_name: "example.com".into(),
        max_ttl: 30,
        full_sweep_every: Duration::from_secs(30),
    }
}

fn p(protocol: Protocol, target: Target) -> Probe {
    Probe { protocol, target }
}

fn start(net: Net, probes: Vec<Probe>, traces: Vec<Probe>) -> (Prober, Receiver<Event>) {
    let group = ProbeGroup {
        name: "test".into(),
        interval: Duration::from_secs(60),
        probes,
        traces,
    };
    Prober::start_with(vec![group], limits(), net)
}

/// Everything one tick sends, waiting briefly for the tick to finish.
fn first_tick(rx: &Receiver<Event>) -> (Vec<ProbeResult>, Vec<HopResult>) {
    let (mut probes, mut hops) = (vec![], vec![]);
    while let Ok(e) = rx.recv_timeout(Duration::from_millis(500)) {
        match e {
            Event::Probe(r) => probes.push(r),
            Event::Hop(h) => hops.push(h),
            Event::Resolved { .. } => {}
        }
    }
    (probes, hops)
}

#[test]
fn a_target_whose_ip_version_has_no_route_is_absent_not_lost() {
    let both = vec![
        p(Protocol::Tcp(443), Target::Addr(V4)),
        p(Protocol::Tcp(443), Target::Addr(V6)),
    ];
    let (_p, rx) = start(
        Net {
            v4: true,
            v6: false,
        },
        both,
        vec![],
    );
    let (probes, _) = first_tick(&rx);
    assert_eq!(probes.len(), 1);
    assert_eq!(probes[0].probe.target, Target::Addr(V4));
    assert_eq!(probes[0].rtt_ms, Some(7.0));
}

#[test]
fn no_path_at_all_sends_nothing() {
    let (_p, rx) = start(
        Net {
            v4: false,
            v6: false,
        },
        vec![p(Protocol::Tcp(443), Target::Addr(V4))],
        vec![],
    );
    assert_eq!(first_tick(&rx), (vec![], vec![]));
}

#[test]
fn an_unresolved_target_counts_as_lost() {
    let name = Target::Name("nowhere.invalid".into());
    let (_p, rx) = start(
        Net { v4: true, v6: true },
        vec![
            p(Protocol::Tcp(443), name.clone()),
            p(Protocol::Icmp, name.clone()),
        ],
        vec![],
    );
    let (probes, _) = first_tick(&rx);
    assert_eq!(probes.len(), 2);
    assert!(probes.iter().all(|r| r.rtt_ms.is_none()));
}

#[test]
fn icmp_probe_and_trace_share_one_batch_and_the_trace_stops_at_the_target() {
    let target = Target::Addr(V4);
    let (_p, rx) = start(
        Net { v4: true, v6: true },
        vec![p(Protocol::Icmp, target.clone())],
        vec![p(Protocol::Icmp, target)],
    );
    let (probes, hops) = first_tick(&rx);
    assert_eq!(probes.len(), 1);
    assert_eq!(probes[0].rtt_ms, Some(128.0));
    let ttls: Vec<u8> = hops.iter().map(|h| h.ttl).collect();
    assert_eq!(ttls, [1, 2, HOPS]);
    assert!(hops.last().unwrap().reached);
    assert_eq!(hops[0].addr, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
}
