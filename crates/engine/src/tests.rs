use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use verdict::FaultDomain;

/// Scripted Sensors: a fixed Sample and network state; counts process reads.
#[derive(Clone, Default)]
struct Fake {
    net: Network,
    process_reads: Arc<AtomicUsize>,
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
}

const AT: u64 = 1_800_000_000;

fn monitor(fake: Fake, idle: bool) -> Monitor<Fake> {
    let config = Config::default();
    let shared = Arc::new(Mutex::new(Shared {
        history: History::open(None, config.history_limits()),
        findings: vec![],
        processes: vec![],
        adapters: vec![],
        paused: false,
        sample_count: 0,
    }));
    let idle = Arc::new(AtomicBool::new(idle));
    Monitor::new(
        config,
        shared,
        fake,
        Box::new(move || idle.load(Ordering::Relaxed)),
    )
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
    assert!(m.tick(t0, AT));
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
    assert!(m.tick(t0, AT));
    assert!(!m.tick(t0 + Duration::from_millis(10), AT));
    assert_eq!(m.shared.lock().unwrap().sample_count, 1);
}

#[test]
fn processes_are_read_only_while_the_panel_is_open() {
    let fake = Fake::default();
    let reads = fake.process_reads.clone();
    let mut m = monitor(fake, false);
    let t0 = Instant::now();
    m.tick(t0, AT);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    m.command(Command::PanelOpen(true));
    m.tick(t0 + Duration::from_secs(5), AT + 5);
    assert_eq!(reads.load(Ordering::Relaxed), 1);
}

#[test]
fn idle_pauses_sampling() {
    let mut m = monitor(Fake::default(), true);
    assert!(!m.tick(Instant::now(), AT));
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
    m.tick(Instant::now(), AT);
    assert!(domains(&m).contains(&FaultDomain::Offline));

    let portal = Fake {
        net: Network {
            captive_portal: true,
            ..Network::default()
        },
        ..Fake::default()
    };
    let mut m = monitor(portal, false);
    m.tick(Instant::now(), AT);
    assert!(domains(&m).contains(&FaultDomain::CaptivePortal));
}
