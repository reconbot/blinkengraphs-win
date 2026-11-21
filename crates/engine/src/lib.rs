use collect::{AdapterChoice, Collector};
use config::{Adapter, Config};
use history::History;
use model::ProcessRow;
use probe::{Event, ProbeGroup, Prober};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};
use verdict::Finding;

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
}

pub struct Engine {
    pub shared: Arc<Mutex<Shared>>,
    pub panel_open: Arc<AtomicBool>,
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
        let history = History::open(
            db.as_deref(),
            config.history.ram_minutes,
            config.history.rollup_days,
        );
        let shared = Arc::new(Mutex::new(Shared {
            history,
            findings: vec![],
            processes: vec![],
            adapters: vec![],
            paused: false,
            sample_count: 0,
        }));
        let panel_open = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (s, p) = (shared.clone(), panel_open.clone());
        thread::Builder::new()
            .name("engine".into())
            .spawn(move || run(config, s, p, rx, wake, idle))
            .expect("spawn engine");
        Self {
            shared,
            panel_open,
            commands: tx,
        }
    }

    pub fn send(&self, c: Command) {
        let _ = self.commands.send(c);
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

fn run(
    mut config: Config,
    shared: Arc<Mutex<Shared>>,
    panel_open: Arc<AtomicBool>,
    commands: Receiver<Command>,
    wake: impl Fn(),
    idle: IdleCheck,
) {
    let mut collector = Collector::new(adapter(&config)).ok();
    let (prober, events) = Prober::start(groups(&config));
    let mut next_sample = Instant::now();
    let mut next_procs = Instant::now();
    let mut next_adapters = Instant::now();
    let mut paused = false;

    loop {
        let timeout = next_sample
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(250));
        match commands.recv_timeout(timeout) {
            Ok(Command::Reconfigure(c)) => {
                if let Some(col) = &mut collector {
                    col.set_adapter(adapter(&c));
                }
                prober.reconfigure(groups(&c));
                shared
                    .lock()
                    .unwrap()
                    .history
                    .set_rollup_days(c.history.rollup_days);
                config = *c;
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }

        let mut probe_events = Vec::new();
        while let Ok(e) = events.try_recv() {
            probe_events.push(e);
        }
        if !probe_events.is_empty() {
            let mut s = shared.lock().unwrap();
            for e in probe_events {
                match e {
                    Event::Probe(r) => s.history.record_probe(&r),
                    Event::Hop(h) => s.history.record_hop(&h),
                    Event::Resolved { .. } => {}
                }
            }
        }

        let now = Instant::now();
        if now < next_sample {
            continue;
        }
        next_sample = now + Duration::from_millis(config.system_interval_ms.max(MIN_INTERVAL_MS));

        let should_pause = config.pause_when_idle && idle();
        if should_pause != paused {
            paused = should_pause;
            prober.set_paused(paused);
            shared.lock().unwrap().paused = paused;
        }
        if paused {
            continue;
        }

        let Some(col) = &mut collector else {
            collector = Collector::new(adapter(&config)).ok();
            continue;
        };
        let mut sample = col.sample();
        sample.at = model::now();
        let panel = panel_open.load(Ordering::Relaxed);
        let procs = (panel && now >= next_procs).then(|| {
            next_procs =
                now + Duration::from_millis(config.process_interval_ms.max(MIN_INTERVAL_MS));
            col.processes(10)
        });
        let adapters = (now >= next_adapters).then(|| {
            next_adapters = now + Duration::from_secs(10);
            col.adapters()
                .into_iter()
                .map(|a| a.alias)
                .collect::<Vec<_>>()
        });

        let mut s = shared.lock().unwrap();
        s.history.record_sample(&sample);
        if let Some(p) = procs {
            s.processes = p;
        }
        if let Some(a) = adapters {
            s.adapters = a;
        }
        s.findings = verdict::from_history(&s.history);
        s.sample_count += 1;
        drop(s);
        wake();
    }
}
