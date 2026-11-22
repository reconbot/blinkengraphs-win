use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use verdict::FaultDomain;

/// Scripted Sensors: a fixed Sample and network state; counts process reads.
#[derive(Clone, Default)]
struct Fake {
    net: Network,
    process_reads: Arc<AtomicUsize>,
    slept: Arc<Mutex<f64>>,
}

impl Sensors for Fake {
    fn sample(&mut self) -> Option<Sample> {
        let mut s = Sample {
            at: 0,
            values: [0.0; Metric::ALL.len()],
            per_core: vec![],
        };
        s.set(Metric::CpuTotal, 0.2);
        Some(s)
    }
    fn processes(&mut self, _: usize) -> Vec<ProcessRow> {
        self.process_reads.fetch_add(1, Ordering::Relaxed);
        vec![]
    }
    fn adapters(&mut self) -> Vec<String> {
        vec!["Ethernet".into()]
    }
    fn set_adapter(&mut self, _: AdapterChoice) {}
    fn network(&mut self) -> Network {
        self.net
    }
    fn slept_s(&mut self) -> f64 {
        *self.slept.lock().unwrap()
    }
}

const AT: u64 = 1_800_000_000;
const WALL: f64 = AT as f64;

fn monitor(fake: Fake, idle: bool) -> Monitor<Fake> {
    let idle = Arc::new(AtomicBool::new(idle));
    monitor_with(fake, Box::new(move || idle.load(Ordering::Relaxed)))
}

fn monitor_with(fake: Fake, idle: IdleCheck) -> Monitor<Fake> {
    let config = Config::default();
    let shared = Arc::new(Mutex::new(Shared {
        history: History::open(None, config.history_limits()),
        findings: vec![],
        processes: vec![],
        adapters: vec![],
        paused: false,
        sample_count: 0,
    }));
    Monitor::new(config, shared, fake, idle)
}

fn domains(m: &Monitor<Fake>) -> Vec<FaultDomain> {
    m.shared
        .lock()
        .unwrap()
        .findings
        .iter()
        .map(|f| f.domain)
        .collect()
}

#[test]
fn a_due_sample_is_recorded_and_judged() {
    let mut m = monitor(Fake::default(), false);
    let t0 = Instant::now();
    assert!(m.tick(t0, WALL));
    let s = m.shared.lock().unwrap();
    assert_eq!(s.sample_count, 1);
    assert_eq!(s.history.last_sample().unwrap().at, AT);
    assert_eq!(s.adapters, ["Ethernet"]);
    assert!(!s.findings.is_empty());
}

#[test]
fn nothing_happens_before_the_next_sample_is_due() {
    let mut m = monitor(Fake::default(), false);
    let t0 = Instant::now();
    assert!(m.tick(t0, WALL));
    assert!(!m.tick(t0 + Duration::from_millis(10), WALL));
    assert_eq!(m.shared.lock().unwrap().sample_count, 1);
}

#[test]
fn processes_are_read_only_while_the_panel_is_open() {
    let fake = Fake::default();
    let reads = fake.process_reads.clone();
    let mut m = monitor(fake, false);
    let t0 = Instant::now();
    m.tick(t0, WALL);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    m.command(Command::PanelOpen(true));
    m.tick(t0 + Duration::from_secs(5), WALL + 5.0);
    assert_eq!(reads.load(Ordering::Relaxed), 1);
}

#[test]
fn idle_pauses_sampling() {
    let mut m = monitor(Fake::default(), true);
    assert!(!m.tick(Instant::now(), WALL));
    let s = m.shared.lock().unwrap();
    assert!(s.paused);
    assert_eq!(s.sample_count, 0);
}

#[test]
fn network_facts_reach_the_verdict() {
    let offline = Fake {
        net: Network {
            path_up: false,
            ..Network::default()
        },
        ..Fake::default()
    };
    let mut m = monitor(offline, false);
    m.tick(Instant::now(), WALL);
    assert!(domains(&m).contains(&FaultDomain::Offline));

    let portal = Fake {
        net: Network {
            captive_portal: true,
            ..Network::default()
        },
        ..Fake::default()
    };
    let mut m = monitor(portal, false);
    m.tick(Instant::now(), WALL);
    assert!(domains(&m).contains(&FaultDomain::CaptivePortal));
}

fn pauses(m: &Monitor<Fake>) -> Vec<history::pause::Pause> {
    m.shared.lock().unwrap().history.pauses()
}

fn settle() -> f64 {
    config::defaults::get().history.settle_s
}

fn round(start: f64, end: f64) -> probe::Round {
    let at = start as u64;
    probe::Round {
        at,
        start,
        end,
        probes: vec![model::ProbeResult {
            probe: Config::default().primary,
            at,
            rtt_ms: Some(12.0),
        }],
        hops: vec![],
    }
}

#[test]
fn idle_rest_is_a_pause_that_ends_a_settle_time_after_measuring_resumes() {
    let idle = Arc::new(AtomicBool::new(false));
    let flag = idle.clone();
    let mut m = monitor_with(
        Fake::default(),
        Box::new(move || flag.load(Ordering::Relaxed)),
    );
    let t0 = Instant::now();
    m.tick(t0, WALL);
    idle.store(true, Ordering::Relaxed);
    m.tick(t0 + Duration::from_secs(1), WALL + 1.0);
    idle.store(false, Ordering::Relaxed);
    m.tick(t0 + Duration::from_secs(60), WALL + 60.0);
    let p = pauses(&m);
    assert_eq!(p.len(), 1);
    assert_eq!((p[0].begin, p[0].end), (WALL + 1.0, WALL + 60.0 + settle()));
}

#[test]
fn suspend_and_resume_bound_a_pause() {
    let mut m = monitor(Fake::default(), false);
    let t0 = Instant::now();
    m.tick(t0, WALL);
    m.command(Command::Suspend(WALL + 0.5));
    assert!(m.shared.lock().unwrap().paused);
    m.command(Command::Resume(WALL + 3600.0));
    assert!(!m.shared.lock().unwrap().paused);
    let p = pauses(&m);
    assert_eq!(
        (p[0].begin, p[0].end),
        (WALL + 0.5, WALL + 3600.0 + settle())
    );
}

#[test]
fn a_round_a_pause_cut_records_nothing() {
    let mut m = monitor(Fake::default(), false);
    m.command(Command::Suspend(WALL + 0.8));
    m.round(round(WALL + 0.1, WALL + 1.1));
    m.command(Command::Resume(WALL + 10.0));
    m.round(round(WALL + 20.1, WALL + 20.2));
    let h = &m.shared.lock().unwrap().history;
    let got = h.recent(&history::Key::Probe(Config::default().primary), 10);
    assert_eq!(got, [Some(12.0)]);
}

#[test]
fn a_sleep_with_no_notice_is_found_and_paused() {
    let fake = Fake::default();
    let slept = fake.slept.clone();
    let mut m = monitor(fake, false);
    let t0 = Instant::now();
    m.tick(t0, WALL);
    *slept.lock().unwrap() = 600.0;
    m.tick(t0 + Duration::from_secs(1), WALL + 601.0);
    let p = pauses(&m);
    assert_eq!(p.len(), 1);
    assert_eq!((p[0].begin, p[0].end), (WALL, WALL + 601.0 + settle()));
}

#[test]
fn the_verdict_says_resuming_after_a_pause() {
    let mut m = monitor(Fake::default(), false);
    let t0 = Instant::now();
    m.command(Command::Suspend(WALL));
    m.command(Command::Resume(WALL + 100.0));
    m.tick(t0, WALL + 101.0);
    assert!(domains(&m).contains(&FaultDomain::Paused));
}

#[test]
fn an_announced_sleep_is_one_pause() {
    let fake = Fake::default();
    let slept = fake.slept.clone();
    let mut m = monitor(fake, false);
    let t0 = Instant::now();
    m.tick(t0, WALL);
    m.command(Command::Suspend(WALL + 1.0));
    *slept.lock().unwrap() = 3600.0;
    m.command(Command::Resume(WALL + 3601.0));
    m.tick(t0 + Duration::from_secs(1), WALL + 3601.5);
    assert_eq!(pauses(&m).len(), 1);
}

#[test]
fn a_resume_with_no_suspend_before_it_still_finds_the_sleep() {
    let fake = Fake::default();
    let slept = fake.slept.clone();
    let mut m = monitor(fake, false);
    let t0 = Instant::now();
    m.tick(t0, WALL);
    *slept.lock().unwrap() = 900.0;
    m.command(Command::Resume(WALL + 901.0));
    m.tick(t0 + Duration::from_secs(1), WALL + 901.5);
    let p = pauses(&m);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].begin, WALL);
}
