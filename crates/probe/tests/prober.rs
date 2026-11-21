use model::{Probe, ProbeResult, Protocol, Target};
use probe::{Event, ProbeGroup, Prober};
use std::{
    net::{IpAddr, Ipv4Addr, TcpListener},
    sync::mpsc::Receiver,
    thread,
    time::{Duration, Instant},
};

const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

fn tcp(port: u16) -> Probe {
    Probe {
        target: Target::Addr(LOCALHOST),
        protocol: Protocol::Tcp(port),
    }
}

fn group(probes: Vec<Probe>, traces: Vec<Probe>) -> ProbeGroup {
    ProbeGroup {
        name: "test".into(),
        interval: Duration::from_millis(100),
        probes,
        traces,
    }
}

fn next_probe(rx: &Receiver<Event>) -> ProbeResult {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left).expect("a probe result") {
            Event::Probe(r) => return r,
            _ => continue,
        }
    }
}

fn drain(rx: &Receiver<Event>) {
    while rx.try_recv().is_ok() {}
}

#[test]
fn tcp_group_ticks() {
    let listener = TcpListener::bind((LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_prober, rx) = Prober::start(vec![group(vec![tcp(port)], vec![])]);
    let start = Instant::now();
    for _ in 0..3 {
        let r = next_probe(&rx);
        assert_eq!(r.probe, tcp(port));
        assert!(r.rtt_ms.is_some());
    }
    assert!(start.elapsed() >= Duration::from_millis(150));
}

#[test]
fn pause_stops_results() {
    let listener = TcpListener::bind((LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (prober, rx) = Prober::start(vec![group(vec![tcp(port)], vec![])]);
    next_probe(&rx);
    prober.set_paused(true);
    drain(&rx);
    thread::sleep(Duration::from_millis(400));
    assert!(rx.try_recv().is_err());
    prober.set_paused(false);
    next_probe(&rx);
}

#[test]
fn reconfigure_swaps_probes() {
    let a = TcpListener::bind((LOCALHOST, 0)).unwrap();
    let b = TcpListener::bind((LOCALHOST, 0)).unwrap();
    let (pa, pb) = (
        a.local_addr().unwrap().port(),
        b.local_addr().unwrap().port(),
    );
    let (prober, rx) = Prober::start(vec![group(vec![tcp(pa)], vec![])]);
    assert_eq!(next_probe(&rx).probe, tcp(pa));
    prober.reconfigure(vec![group(vec![tcp(pb)], vec![])]);
    drain(&rx);
    for _ in 0..3 {
        assert_eq!(next_probe(&rx).probe, tcp(pb));
    }
}

#[test]
fn host_resolution_is_reported() {
    let probe = Probe {
        target: Target::Name("localhost".into()),
        protocol: Protocol::Tcp(1),
    };
    let (_prober, rx) = Prober::start(vec![group(vec![probe], vec![])]);
    match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::Resolved { target, addr } => {
            assert_eq!(target, Target::Name("localhost".into()));
            assert!(addr.is_some_and(|a| a.is_loopback()));
        }
        e => panic!("expected Resolved first, got {e:?}"),
    }
}

#[cfg(windows)]
#[test]
fn icmp_to_localhost_replies() {
    let icmp = Probe {
        target: Target::Addr(LOCALHOST),
        protocol: Protocol::Icmp,
    };
    let (_prober, rx) = Prober::start(vec![group(vec![icmp.clone()], vec![icmp.clone()])]);
    let r = next_probe(&rx);
    assert_eq!(r.probe, icmp);
    assert!(r.rtt_ms.is_some());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if let Event::Hop(h) = rx.recv_timeout(left).expect("a hop") {
            assert_eq!(h.ttl, 1);
            assert!(h.reached);
            break;
        }
    }
}
