//! The Monitor: samples the system through [`Sensors`], runs the Prober, records both into
//! History and keeps the Verdict current. Surfaces read [`Shared`] and send [`Command`]s.

use collect::AdapterChoice;
use config::{Adapter, Config};
use history::History;
use model::{Metric, Probe, ProbeStat, ProcessRow, Sample};
use probe::{Event, ProbeGroup, Prober};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};
use verdict::Finding;

mod sensors;
pub use sensors::{Network, Sensors, SystemSensors};

/// Faster than this starves the UI thread for no visible gain.
const MIN_INTERVAL_MS: u64 = 250;

pub struct Shared {
    pub history: History,
    pub findings: Vec<Finding>,
    pub processes: Vec<ProcessRow>,
    pub adapters: Vec<String>,
    pub paused: bool,
    /// Bumped on every new Sample so Surfaces redraw once per Sample.
    pub sample_count: u64,
}

pub enum Command {
    Reconfigure(Box<Config>),
    /// Process rows are collected only while the Panel is open (ADR 0003).
    PanelOpen(bool),
    /// Save what a restart would lose, reply on the channel, then stop.
    Shutdown(Sender<()>),
}

pub struct Engine {
    pub shared: Arc<Mutex<Shared>>,
    commands: Sender<Command>,
}

/// True while sampling should pause (session locked, display off, screen saver).
pub type IdleCheck = Box<dyn Fn() -> bool + Send>;

impl Engine {
    pub fn start(
        config: Config,
        db: Option<PathBuf>,
        wake: impl Fn() + Send + 'static,
        idle: IdleCheck,
    ) -> Self {
        let sensors = SystemSensors::new(adapter(&config));
        Self::start_with(config, db, sensors, wake, idle)
    }

    pub fn start_with(
        config: Config,
        db: Option<PathBuf>,
        sensors: impl Sensors + 'static,
        wake: impl Fn() + Send + 'static,
        idle: IdleCheck,
    ) -> Self {
        let history = History::open(db.as_deref(), config.history_limits());
        let shared = Arc::new(Mutex::new(Shared {
            history,
            findings: vec![],
            processes: vec![],
            adapters: vec![],
            paused: false,
            sample_count: 0,
        }));
        let (tx, rx) = channel();
        let s = shared.clone();
        thread::Builder::new()
            .name("engine".into())
            .spawn(move || {
                let mut m = Monitor::new(config, s, sensors, idle);
                run(&mut m, rx, wake)
            })
            .expect("spawn engine");
        Self {
            shared,
            commands: tx,
        }
    }

    pub fn send(&self, c: Command) {
        let _ = self.commands.send(c);
    }

    /// Waits up to `timeout` for the engine to save history; a stuck engine must not block Quit.
    pub fn shutdown(&self, timeout: Duration) -> bool {
        let (tx, rx) = channel();
        self.send(Command::Shutdown(tx));
        rx.recv_timeout(timeout).is_ok()
    }
}

/// Each configured group runs as up to three Prober groups, one per collection, each on its own interval.
pub fn groups(c: &Config) -> Vec<ProbeGroup> {
    let ms = |v: u64| Duration::from_millis(v.max(MIN_INTERVAL_MS));
    let mut gs: Vec<ProbeGroup> = Vec::new();
    for g in c.groups.iter().filter(|g| g.enabled) {
        let parts = [
            ("latency", g.latency_ms, g.latency_probes(), vec![]),
            ("dns", g.dns_ms, g.dns_probes(), vec![]),
            ("trace", g.trace_ms, vec![], g.traces()),
        ];
        for (kind, interval, probes, traces) in parts {
            if !probes.is_empty() || !traces.is_empty() {
                gs.push(ProbeGroup {
                    name: format!("{} {kind}", g.name),
                    interval: ms(interval),
                    probes,
                    traces,
                });
            }
        }
    }
    if !gs.iter().any(|g| g.probes.contains(&c.primary)) {
        gs.push(ProbeGroup {
            name: "Latency Meter".into(),
            interval: Duration::from_secs(1),
            probes: vec![c.primary.clone()],
            traces: vec![],
        });
    }
    gs
}

fn adapter(c: &Config) -> AdapterChoice {
    match &c.adapter {
        Adapter::FollowDefaultRoute => AdapterChoice::FollowDefaultRoute,
        Adapter::Pinned(a) => AdapterChoice::Pinned(a.clone()),
    }
}

/// The Verdict for what History holds now plus what the OS and network checks report
/// (spec `rules/verdict.md` inputs): recent Probe stats, the last Sample, the upload peak.
pub fn diagnose(h: &History, net: &Network) -> Vec<Finding> {
    let probes: Vec<(Probe, ProbeStat)> = h
        .probes()
        .into_iter()
        .map(|p| {
            let st = h.now(&p);
            (p, st)
        })
        .collect();
    let peak = h
        .recent(&history::Key::Metric(Metric::NetUp), usize::MAX)
        .into_iter()
        .flatten()
        .fold(0.0f32, f32::max) as f64;
    verdict::diagnose(&verdict::Input {
        sample: h.last_sample(),
        probes: &probes,
        upload_peak_bps: peak,
        path_up: net.path_up,
        via_vpn: net.via_vpn,
        captive_portal: net.captive_portal,
    })
}

fn run<S: Sensors>(m: &mut Monitor<S>, commands: Receiver<Command>, wake: impl Fn()) {
    loop {
        let timeout = m
            .next_sample
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(250));
        match commands.recv_timeout(timeout) {
            Ok(Command::Shutdown(done)) => {
                m.shared.lock().unwrap().history.close(model::now());
                let _ = done.send(());
                return;
            }
            Ok(c) => m.command(c),
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if m.tick(Instant::now(), model::now()) {
            wake();
        }
    }
}

struct Monitor<S> {
    config: Config,
    shared: Arc<Mutex<Shared>>,
    sensors: S,
    prober: Prober,
    events: Receiver<Event>,
    idle: IdleCheck,
    panel_open: bool,
    paused: bool,
    next_sample: Instant,
    next_procs: Instant,
    next_adapters: Instant,
}

impl<S: Sensors> Monitor<S> {
    fn new(config: Config, shared: Arc<Mutex<Shared>>, sensors: S, idle: IdleCheck) -> Self {
        let (prober, events) = Prober::start(groups(&config), config.probe_limits());
        let now = Instant::now();
        Self {
            config,
            shared,
            sensors,
            prober,
            events,
            idle,
            panel_open: false,
            paused: false,
            next_sample: now,
            next_procs: now,
            next_adapters: now,
        }
    }

    fn command(&mut self, c: Command) {
        match c {
            Command::Reconfigure(c) => {
                self.sensors.set_adapter(adapter(&c));
                self.prober.reconfigure(groups(&c));
                self.shared
                    .lock()
                    .unwrap()
                    .history
                    .set_rollup_days(c.history.rollup_days);
                self.config = *c;
            }
            Command::PanelOpen(open) => self.panel_open = open,
            Command::Shutdown(_) => {}
        }
    }

    /// Records Probe results, then, when a Sample is due, takes it and updates the Verdict.
    /// True when Surfaces should redraw.
    fn tick(&mut self, now: Instant, at: u64) -> bool {
        let events: Vec<Event> = self.events.try_iter().collect();
        if !events.is_empty() {
            let mut s = self.shared.lock().unwrap();
            for e in events {
                match e {
                    Event::Probe(r) => s.history.record_probe(&r),
                    Event::Hop(h) => s.history.record_hop(&h),
                    Event::Resolved { .. } => {}
                }
            }
        }
        if now < self.next_sample {
            return false;
        }
        let every = |ms: u64| Duration::from_millis(ms.max(MIN_INTERVAL_MS));
        self.next_sample = now + every(self.config.system_interval_ms);

        let pause = self.config.pause_when_idle && (self.idle)();
        if pause != self.paused {
            self.paused = pause;
            self.prober.set_paused(pause);
            self.shared.lock().unwrap().paused = pause;
        }
        if pause {
            return false;
        }
        let Some(mut sample): Option<Sample> = self.sensors.sample() else {
            return false;
        };
        sample.at = at;
        let procs = (self.panel_open && now >= self.next_procs).then(|| {
            self.next_procs = now + every(self.config.process_interval_ms);
            self.sensors.processes(10)
        });
        let adapters = (now >= self.next_adapters).then(|| {
            self.next_adapters = now + Duration::from_secs(10);
            self.sensors.adapters()
        });
        let net = self.sensors.network();

        let mut s = self.shared.lock().unwrap();
        s.history.record_sample(&sample);
        if let Some(p) = procs {
            s.processes = p;
        }
        if let Some(a) = adapters {
            s.adapters = a;
        }
        s.findings = diagnose(&s.history, &net);
        s.sample_count += 1;
        true
    }
}

#[cfg(test)]
mod tests;
