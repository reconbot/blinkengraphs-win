//! The Prober's spec rules (`rules/probe-stats.md`, `rules/hop-trace.md`) over a scripted network.

use model::{HopResult, Link, LinkId, LinkKind, Probe, ProbeResult, Protocol, Target};
use probe::{Echo, Event, Limits, ProbeGroup, Prober, Transport, transport::Hop};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex, mpsc::Receiver},
    time::Duration,
};

const V4: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
const V6: IpAddr = IpAddr::V6(std::net::Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
/// The target in the scripted network is this many hops away.
const HOPS: u8 = 3;

/// Each send: the Link's NET_LUID it went out on, and where to.
type Sent = Arc<Mutex<Vec<(Option<u64>, IpAddr)>>>;

#[derive(Default)]
struct Net {
    v4: bool,
    v6: bool,
    /// Destinations a Link doesn't route.
    unrouted: Vec<IpAddr>,
    sent: Sent,
}

impl Net {
    fn up() -> Net {
        Net {
            v4: true,
            v6: true,
            ..Net::default()
        }
    }
    fn send(&self, via: Option<&Link>, to: IpAddr) {
        self.sent.lock().unwrap().push((via.map(|l| l.id.luid), to));
    }
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
    fn routes(&self, _: &Link, to: IpAddr) -> bool {
        !self.unrouted.contains(&to)
    }
    fn tcp(&self, via: Option<&Link>, to: SocketAddr, _: Duration) -> Option<Duration> {
        self.send(via, to.ip());
        Some(Duration::from_millis(7))
    }
    fn udp(&self, via: Option<&Link>, to: SocketAddr, _: Duration) -> Option<Duration> {
        self.send(via, to.ip());
        None
    }
    fn dns(&self, via: Option<&Link>, to: SocketAddr, _: Duration, _: &str) -> Option<Duration> {
        self.send(via, to.ip());
        Some(Duration::from_millis(9))
    }
    fn echo(
        &self,
        via: Option<&Link>,
        requests: &[(IpAddr, u8)],
        _: Duration,
    ) -> Option<Vec<Option<Echo>>> {
        for (to, _) in requests {
            self.send(via, *to);
        }
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
    fn tcp_trace(
        &self,
        _: Option<&Link>,
        _: Ipv4Addr,
        _: u16,
        _: u8,
        _: Duration,
    ) -> Option<Vec<Hop>> {
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
    start_on(net, None, probes, traces)
}

fn start_on(
    net: Net,
    link: Option<Link>,
    probes: Vec<Probe>,
    traces: Vec<Probe>,
) -> (Prober, Receiver<Event>) {
    let group = ProbeGroup {
        name: "test".into(),
        interval: Duration::from_secs(60),
        probes,
        traces,
        link,
    };
    Prober::start_with(vec![group], limits(), net)
}

/// Everything one tick sends, waiting briefly for the tick to finish.
fn first_tick(rx: &Receiver<Event>) -> (Vec<ProbeResult>, Vec<HopResult>) {
    let (mut probes, mut hops) = (vec![], vec![]);
    while let Ok(e) = rx.recv_timeout(Duration::from_millis(500)) {
        if let Event::Round(r) = e {
            probes.extend(r.probes);
            hops.extend(r.hops);
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
            ..Net::default()
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
            ..Net::default()
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
        Net::up(),
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
        Net::up(),
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

#[test]
fn a_round_arrives_whole_stamped_with_its_start_and_end() {
    let icmp = p(Protocol::Icmp, Target::Addr(V4));
    let tcp = p(Protocol::Tcp(443), Target::Addr(V4));
    let (_prober, rx) = start(
        Net {
            v4: true,
            v6: false,
            ..Net::default()
        },
        vec![icmp, tcp],
        vec![],
    );
    let round = loop {
        if let Event::Round(r) = rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            break r;
        }
    };
    assert_eq!(round.probes.len(), 2);
    assert!(round.start <= round.end);
    assert_eq!(round.at, round.start.floor() as u64);
    assert!(round.probes.iter().all(|r| r.at == round.at));
}

fn wifi() -> Link {
    Link {
        id: LinkId {
            luid: 7,
            gateway: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))),
        },
        alias: "Wi-Fi".into(),
        kind: LinkKind::WiFi,
        index_v4: 12,
        index_v6: 12,
        source_v4: Some(Ipv4Addr::new(192, 168, 1, 20)),
        source_v6: None,
        default_route: true,
    }
}

fn round(rx: &Receiver<Event>) -> probe::Round {
    loop {
        if let Event::Round(r) = rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            return r;
        }
    }
}

#[test]
fn a_link_s_probes_leave_from_it_and_its_gateway_is_its_own() {
    let net = Net::up();
    let sent = net.sent.clone();
    let (_p, rx) = start_on(
        net,
        Some(wifi()),
        vec![
            p(Protocol::Tcp(443), Target::Addr(V4)),
            p(Protocol::Icmp, Target::Gateway),
        ],
        vec![],
    );
    let r = round(&rx);
    assert_eq!(r.link, Some(wifi().id));
    assert_eq!(r.probes.len(), 2);
    let mut sent = sent.lock().unwrap().clone();
    sent.sort();
    assert_eq!(
        sent,
        [
            (Some(7), V4),
            (Some(7), IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
        ]
    );
}

#[test]
fn a_target_its_link_does_not_route_is_absent_not_lost() {
    let net = Net {
        unrouted: vec![V4],
        ..Net::up()
    };
    let (_p, rx) = start_on(
        net,
        Some(wifi()),
        vec![
            p(Protocol::Tcp(443), Target::Addr(V4)),
            p(Protocol::Tcp(443), Target::Addr(V6)),
        ],
        vec![],
    );
    let r = round(&rx);
    let to: Vec<&Target> = r.probes.iter().map(|r| &r.probe.target).collect();
    assert_eq!(to, [&Target::Addr(V6)]);
}
