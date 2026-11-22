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
    /// The Links being measured now.
    pub links: Vec<model::Link>,
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
    /// The machine is about to sleep, at this Unix time: a Pause begins.
    Suspend(f64),
    /// The machine woke at this Unix time: measuring resumes, and the Pause ends a settle time later.
    Resume(f64),
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
        let url = config::defaults::get().captive_portal_url;
        let sensors = SystemSensors::new(adapter(&config), url);
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
            links: vec![],
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

    /// A handle for threads that send commands themselves, such as power notifications.
    pub fn commands(&self) -> Sender<Command> {
        self.commands.clone()
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
                    link: None,
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
            link: None,
        });
    }
    gs
}

/// Each group of `groups` on each Link, or once with the OS choosing when no Links are known.
pub fn groups_on(c: &Config, links: &[model::Link]) -> Vec<ProbeGroup> {
    let base = groups(c);
    if links.is_empty() {
        return base;
    }
    links
        .iter()
        .flat_map(|l| {
            base.iter().map(move |g| ProbeGroup {
                name: format!("{} on {}", g.name, l.alias),
                link: Some(l.clone()),
                ..g.clone()
            })
        })
        .collect()
}

/// The Followed Adapter's Link: the pinned adapter's, else the one carrying the default route
/// (spec `rules/verdict.md`, Input).
pub fn followed(c: &Config, links: &[model::Link]) -> Option<model::LinkId> {
    let link = match &c.adapter {
        Adapter::Pinned(alias) => links.iter().find(|l| &l.alias == alias),
        Adapter::FollowDefaultRoute => None,
    }
    .or_else(|| links.iter().find(|l| l.default_route))
    .or_else(|| links.first());
    link.map(|l| l.id.clone())
}

fn adapter(c: &Config) -> AdapterChoice {
    match &c.adapter {
        Adapter::FollowDefaultRoute => AdapterChoice::FollowDefaultRoute,
        Adapter::Pinned(a) => AdapterChoice::Pinned(a.clone()),
    }
}

/// The Verdict for what History holds at `now` plus what the OS and network checks report
/// (spec `rules/verdict.md` inputs): Probe stats in `order`, the last Sample, the upload peak.
/// `order` is Probe Group order, so a tie names the Target listed first; Probes absent from
/// it follow. Probes with an empty window have no stats and are left out.
pub fn diagnose(h: &History, net: &Network, now: u64, order: &[Probe]) -> Vec<Finding> {
    let since_pause_s = h.since_pause_s(now as f64);
    let mut probes: Vec<(Probe, ProbeStat)> = h
        .probes()
        .into_iter()
        .filter_map(|p| {
            let st = h.now(&p, now)?;
            Some((p, st))
        })
        .collect();
    probes.sort_by_key(|(p, _)| {
        (
            order.iter().position(|o| o == p).unwrap_or(usize::MAX),
            p.to_string(),
        )
    });
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
        since_pause_s,
        window_s: h.window_s(),
    })
}

/// Every Probe in Probe Group order.
pub fn probe_order(c: &Config) -> Vec<Probe> {
    let mut out: Vec<Probe> = Vec::new();
    for p in groups(c).into_iter().flat_map(|g| g.probes) {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

fn run<S: Sensors>(m: &mut Monitor<S>, commands: Receiver<Command>, wake: impl Fn()) {
    loop {
        let timeout = m
            .next_sample
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(250));
        match commands.recv_timeout(timeout) {
            Ok(Command::Shutdown(done)) => {
                let mut s = m.shared.lock().unwrap();
                // Not measuring from here to the next launch: a Pause.
                s.history.begin_pause(model::now_f64());
                s.history.close(model::now());
                drop(s);
                let _ = done.send(());
                return;
            }
            Ok(c) => m.command(c),
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if m.tick(Instant::now(), model::now_f64()) {
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
    /// Resting while idle (display off, locked, screen saver).
    resting: bool,
    /// Between a suspend and the resume.
    suspended: bool,
    /// The last reading of `Sensors::slept_s`, and when it was taken.
    slept: Option<(f64, f64)>,
    settle_s: f64,
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
            resting: false,
            suspended: false,
            slept: None,
            settle_s: config::defaults::get().history.settle_s,
            next_sample: now,
            next_procs: now,
            next_adapters: now,
        }
    }

    fn command(&mut self, c: Command) {
        match c {
            Command::Reconfigure(c) => {
                self.sensors.set_adapter(adapter(&c));
                let mut s = self.shared.lock().unwrap();
                self.prober.reconfigure(groups_on(&c, &s.links));
                s.history.set_archive_days(c.history.archive_days);
                let f = followed(&c, &s.links);
                s.history.set_followed(f, model::now());
                drop(s);
                self.config = *c;
            }
            Command::PanelOpen(open) => self.panel_open = open,
            Command::Shutdown(_) => {}
            Command::Suspend(at) => {
                if self.measuring() {
                    self.stop(at);
                }
                self.suspended = true;
            }
            Command::Resume(at) => {
                // The time asleep is this Pause's, not a missed one. A resume with no suspend
                // before it (a critical one) keeps the reading, so the next tick finds the sleep.
                if self.suspended {
                    self.slept = None;
                }
                self.suspended = false;
                if self.measuring() {
                    self.start(at);
                }
            }
        }
    }

    fn measuring(&self) -> bool {
        !self.resting && !self.suspended
    }

    /// Measuring stops at `at`: a Pause begins and the Prober rests.
    fn stop(&mut self, at: f64) {
        self.prober.set_paused(true);
        let mut s = self.shared.lock().unwrap();
        s.history.begin_pause(at);
        s.paused = true;
    }

    /// Measuring resumes at `at`; the Pause lasts the settle time more, while the network
    /// comes back (spec `rules/history.md`, Pauses).
    fn start(&mut self, at: f64) {
        self.prober.set_paused(false);
        let mut s = self.shared.lock().unwrap();
        s.history.end_pause(at + self.settle_s);
        s.paused = false;
    }

    /// Records a Round's results, unless a Pause cut it.
    fn round(&mut self, r: probe::Round) {
        let mut s = self.shared.lock().unwrap();
        if s.history.cut(r.start, r.end) {
            return;
        }
        s.history.record_round(r.link.as_ref(), &r.probes, &r.hops);
    }

    /// Takes up the Links up now: on a change, every Probe Group runs on each of them, and
    /// History follows the Followed Adapter's.
    fn update_links(&mut self, links: Vec<model::Link>, at: u64) {
        let mut s = self.shared.lock().unwrap();
        if links != s.links {
            self.prober.reconfigure(groups_on(&self.config, &links));
            s.links = links;
        }
        let f = followed(&self.config, &s.links);
        s.history.set_followed(f, at);
    }

    /// A suspend Windows never announced, such as a critical one, shows as time asleep that
    /// grew since the last tick; that stretch becomes a Pause after the fact.
    fn catch_missed_sleep(&mut self, wall: f64) {
        let slept = self.sensors.slept_s();
        if let Some((before, last)) = self.slept
            && slept - before > 1.0
            && self.measuring()
        {
            let mut s = self.shared.lock().unwrap();
            s.history.begin_pause(last);
            s.history.end_pause(wall + self.settle_s);
        }
        self.slept = Some((slept, wall));
    }

    /// Records Probe results, then, when a Sample is due, takes it and updates the Verdict.
    /// True when Surfaces should redraw.
    fn tick(&mut self, now: Instant, wall: f64) -> bool {
        let at = wall as u64;
        self.catch_missed_sleep(wall);
        let events: Vec<Event> = self.events.try_iter().collect();
        for e in events {
            if let Event::Round(r) = e {
                self.round(r);
            }
        }
        if now < self.next_sample {
            return false;
        }
        let every = |ms: u64| Duration::from_millis(ms.max(MIN_INTERVAL_MS));
        self.next_sample = now + every(self.config.system_interval_ms);

        let rest = self.config.pause_when_idle && (self.idle)();
        if rest != self.resting {
            let was = self.measuring();
            self.resting = rest;
            match (was, self.measuring()) {
                (true, false) => self.stop(wall),
                (false, true) => self.start(wall),
                _ => {}
            }
        }
        if !self.measuring() {
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
            let links = self.sensors.links();
            self.update_links(links, at);
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
        s.findings = diagnose(&s.history, &net, at, &probe_order(&self.config));
        s.sample_count += 1;
        true
    }
}

#[cfg(test)]
mod tests;
