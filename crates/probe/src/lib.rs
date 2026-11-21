//! Runs Probe Groups on their own threads and streams results over a channel. No admin rights needed (ADR 0002).

mod dns;
mod net;
mod resolve;
#[cfg_attr(not(windows), allow(dead_code))]
mod trace;
#[cfg(windows)]
mod win;

use model::{HopResult, Probe, ProbeResult, Protocol, Target};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{Receiver, Sender, channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use trace::{Hop, PathMemory};

const MAX_TIMEOUT: Duration = Duration::from_secs(1);
const MIN_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Debug)]
pub struct ProbeGroup {
    pub name: String,
    pub interval: Duration,
    pub probes: Vec<Probe>,
    pub traces: Vec<Probe>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Probe(ProbeResult),
    Hop(HopResult),
    Resolved {
        target: Target,
        addr: Option<IpAddr>,
    },
}

pub struct Prober {
    shared: Arc<Shared>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

struct Control {
    generation: u64,
    paused: bool,
    stopped: bool,
}

struct Shared {
    control: Mutex<Control>,
    wake: Condvar,
    tx: Sender<Event>,
    resolver: resolve::Resolver,
}

impl Prober {
    pub fn start(groups: Vec<ProbeGroup>) -> (Prober, Receiver<Event>) {
        let (tx, rx) = channel();
        let prober = Prober {
            shared: Arc::new(Shared {
                control: Mutex::new(Control {
                    generation: 0,
                    paused: false,
                    stopped: false,
                }),
                wake: Condvar::new(),
                tx,
                resolver: Default::default(),
            }),
            threads: Mutex::new(Vec::new()),
        };
        prober.spawn(0, groups);
        (prober, rx)
    }

    /// No Probe or Hop event from the old groups arrives after this returns.
    pub fn reconfigure(&self, groups: Vec<ProbeGroup>) {
        let generation = {
            let mut c = self.shared.control.lock().unwrap();
            c.generation += 1;
            c.generation
        };
        self.shared.wake.notify_all();
        self.spawn(generation, groups);
    }

    /// No Probe or Hop event arrives after pausing returns.
    pub fn set_paused(&self, paused: bool) {
        self.shared.control.lock().unwrap().paused = paused;
        self.shared.wake.notify_all();
    }

    fn spawn(&self, generation: u64, groups: Vec<ProbeGroup>) {
        let mut threads = self.threads.lock().unwrap();
        threads.retain(|t| !t.is_finished());
        for group in groups {
            let shared = self.shared.clone();
            let handle = thread::Builder::new()
                .name(format!("probe {}", group.name))
                .spawn(move || run(&shared, generation, group))
                .expect("spawn probe thread");
            threads.push(handle);
        }
    }
}

impl Drop for Prober {
    fn drop(&mut self) {
        self.shared.control.lock().unwrap().stopped = true;
        self.shared.wake.notify_all();
        for t in self.threads.get_mut().unwrap().drain(..) {
            let _ = t.join();
        }
    }
}

impl Shared {
    fn emit(&self, generation: u64, event: Event) {
        let c = self.control.lock().unwrap();
        if !c.stopped && !c.paused && c.generation == generation {
            let _ = self.tx.send(event);
        }
    }

    fn resolve(&self, target: &Target) -> Option<IpAddr> {
        self.resolver.resolve(target, |addr| {
            if !self.control.lock().unwrap().stopped {
                let _ = self.tx.send(Event::Resolved {
                    target: target.clone(),
                    addr,
                });
            }
        })
    }
}

fn run(shared: &Shared, generation: u64, group: ProbeGroup) {
    let interval = group.interval.max(MIN_INTERVAL);
    let mut ticker = Ticker::new(group);
    let mut next = Instant::now();
    loop {
        {
            let mut c = shared.control.lock().unwrap();
            loop {
                if c.stopped || c.generation != generation {
                    return;
                }
                if c.paused {
                    c = shared.wake.wait(c).unwrap();
                    next = Instant::now();
                    continue;
                }
                let now = Instant::now();
                if now >= next {
                    break;
                }
                c = shared.wake.wait_timeout(c, next - now).unwrap().0;
            }
        }
        ticker.tick(shared, generation, interval.min(MAX_TIMEOUT));
        next += interval;
        next = next.max(Instant::now());
    }
}

struct Ticker {
    group: ProbeGroup,
    paths: Vec<PathMemory>,
    #[cfg(windows)]
    icmp: Option<win::Icmp>,
}

impl Ticker {
    fn new(group: ProbeGroup) -> Self {
        Ticker {
            paths: group.traces.iter().map(|_| PathMemory::default()).collect(),
            group,
            #[cfg(windows)]
            icmp: win::Icmp::new(),
        }
    }

    fn tick(&mut self, shared: &Shared, generation: u64, timeout: Duration) {
        let at = model::now();
        let mut addrs: HashMap<&Target, Option<IpAddr>> = HashMap::new();
        for p in self.group.probes.iter().chain(&self.group.traces) {
            if !addrs.contains_key(&p.target) {
                addrs.insert(&p.target, shared.resolve(&p.target));
            }
        }
        let emit_probe = |probe: &Probe, rtt: Option<Duration>| {
            let result = ProbeResult {
                probe: probe.clone(),
                at,
                rtt_ms: rtt.map(net::ms),
            };
            shared.emit(generation, Event::Probe(result));
        };
        let emit_hops = |trace: &Probe, hops: Vec<Hop>| {
            for h in hops {
                let hop = HopResult {
                    trace: trace.clone(),
                    at,
                    ttl: h.ttl,
                    addr: h.addr,
                    rtt_ms: h.rtt_ms,
                    reached: h.reached,
                };
                shared.emit(generation, Event::Hop(hop));
            }
        };

        let (icmp_paths, tcp_paths): (Vec<_>, Vec<_>) = self
            .group
            .traces
            .iter()
            .zip(self.paths.iter_mut())
            .partition(|(t, _)| t.protocol == Protocol::Icmp);

        thread::scope(|s| {
            for probe in &self.group.probes {
                let addr = addrs[&probe.target];
                let port = match probe.protocol {
                    Protocol::Tcp(port) => port,
                    Protocol::Dns => 53,
                    Protocol::Icmp => continue,
                };
                let emit_probe = &emit_probe;
                s.spawn(move || {
                    let rtt = addr.and_then(|ip| {
                        let sa = SocketAddr::new(ip, port);
                        match probe.protocol {
                            Protocol::Dns => net::dns(sa, timeout),
                            _ => net::tcp(sa, timeout),
                        }
                    });
                    emit_probe(probe, rtt);
                });
            }

            #[cfg(windows)]
            for (trace, path) in tcp_paths {
                let Protocol::Tcp(port) = trace.protocol else {
                    continue;
                };
                let Some(IpAddr::V4(ip)) = addrs[&trace.target] else {
                    continue;
                };
                let emit_hops = &emit_hops;
                s.spawn(move || {
                    let now = Instant::now();
                    let swept = path.max_ttl(now);
                    let hops = win::tcp_trace(ip, port, swept, timeout);
                    emit_hops(trace, path.record(swept, hops, now));
                });
            }
            #[cfg(not(windows))]
            let _ = (tcp_paths, &emit_hops);

            #[cfg(windows)]
            if let Some(icmp) = &self.icmp {
                icmp_tick(
                    icmp,
                    &self.group.probes,
                    icmp_paths,
                    &addrs,
                    timeout,
                    &emit_probe,
                    &emit_hops,
                );
            }
            #[cfg(not(windows))]
            let _ = icmp_paths;
        });
    }
}

#[cfg(windows)]
fn icmp_tick(
    icmp: &win::Icmp,
    probes: &[Probe],
    traces: Vec<(&Probe, &mut PathMemory)>,
    addrs: &HashMap<&Target, Option<IpAddr>>,
    timeout: Duration,
    emit_probe: &impl Fn(&Probe, Option<Duration>),
    emit_hops: &impl Fn(&Probe, Vec<Hop>),
) {
    use std::net::Ipv4Addr;

    enum Tag {
        Probe(usize),
        Hop(usize, u8),
    }
    let mut requests: Vec<(Ipv4Addr, u8)> = Vec::new();
    let mut tags = Vec::new();
    for (i, probe) in probes.iter().enumerate() {
        if probe.protocol != Protocol::Icmp {
            continue;
        }
        match addrs[&probe.target] {
            Some(IpAddr::V4(ip)) => {
                requests.push((ip, 128));
                tags.push(Tag::Probe(i));
            }
            Some(IpAddr::V6(_)) => {}
            None => emit_probe(probe, None),
        }
    }
    let now = Instant::now();
    let mut swept = Vec::new();
    for (i, (trace, path)) in traces.iter().enumerate() {
        let max = path.max_ttl(now);
        swept.push(max);
        if let Some(IpAddr::V4(ip)) = addrs[&trace.target] {
            for ttl in 1..=max {
                requests.push((ip, ttl));
                tags.push(Tag::Hop(i, ttl));
            }
        }
    }
    if requests.is_empty() {
        return;
    }

    let replies = icmp.batch(&requests, timeout);
    let mut hops: Vec<Vec<Hop>> = traces.iter().map(|_| Vec::new()).collect();
    for (tag, reply) in tags.into_iter().zip(replies) {
        match tag {
            Tag::Probe(i) => emit_probe(
                &probes[i],
                reply.filter(|r| r.status == win::IP_SUCCESS).map(|r| r.rtt),
            ),
            Tag::Hop(i, ttl) => hops[i].push(Hop {
                ttl,
                addr: reply.as_ref().map(|r| r.from.into()),
                rtt_ms: reply.as_ref().map(|r| net::ms(r.rtt)),
                reached: reply.is_some_and(|r| r.status == win::IP_SUCCESS),
            }),
        }
    }
    for (((trace, path), hops), swept) in traces.into_iter().zip(hops).zip(swept) {
        if !hops.is_empty() {
            emit_hops(trace, path.record(swept, hops, now));
        }
    }
}
