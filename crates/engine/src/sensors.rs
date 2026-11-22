use collect::{AdapterChoice, Collector};
use model::{ProcessRow, Sample};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

/// What the network looks like to the OS, beyond what Probes measure (spec `rules/verdict.md`
/// inputs `path_up`, `via_vpn`, `captive_portal`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Network {
    pub path_up: bool,
    pub via_vpn: bool,
    pub captive_portal: bool,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            path_up: true,
            via_vpn: false,
            captive_portal: false,
        }
    }
}

/// Where the Monitor reads the machine: the OS adapter, or a scripted one in tests.
pub trait Sensors: Send {
    /// None while the counters can't be read yet.
    fn sample(&mut self) -> Option<Sample>;
    fn processes(&mut self, limit: usize) -> Vec<ProcessRow>;
    /// Adapter aliases the user can pin.
    fn adapters(&mut self) -> Vec<String>;
    fn set_adapter(&mut self, a: AdapterChoice);
    fn network(&mut self) -> Network;
    /// Seconds asleep since boot; a jump is a suspend that sent no notice.
    fn slept_s(&mut self) -> f64;
}

/// The OS adapter: performance counters through `collect`, network state through `probe`.
pub struct SystemSensors {
    adapter: AdapterChoice,
    collector: Option<Collector>,
    captive: Arc<AtomicBool>,
}

impl SystemSensors {
    /// `captive_portal_url` is `defaults.json`'s; it is checked on a thread of its own.
    pub fn new(adapter: AdapterChoice, captive_portal_url: String) -> Self {
        let captive = Arc::new(AtomicBool::new(false));
        let flag = captive.clone();
        thread::Builder::new()
            .name("captive portal".into())
            .spawn(move || watch_captive(&captive_portal_url, &flag))
            .expect("spawn captive portal check");
        Self {
            collector: Collector::new(adapter.clone()).ok(),
            adapter,
            captive,
        }
    }
}

/// A portal shows up when joining a network and clears when the user signs in, so check
/// often while one is up and rarely otherwise. No answer counts as no portal, the spec's
/// default for `captive_portal`.
fn watch_captive(url: &str, captive: &AtomicBool) {
    loop {
        let c =
            probe::path_up() && probe::captive::check(url, Duration::from_secs(5)).unwrap_or(false);
        captive.store(c, Ordering::Relaxed);
        thread::sleep(Duration::from_secs(if c { 15 } else { 60 }));
    }
}

impl Sensors for SystemSensors {
    fn sample(&mut self) -> Option<Sample> {
        // Counters can fail to open early in a session; keep trying each Sample.
        if self.collector.is_none() {
            self.collector = Collector::new(self.adapter.clone()).ok();
            return None;
        }
        self.collector.as_mut().map(Collector::sample)
    }

    fn processes(&mut self, limit: usize) -> Vec<ProcessRow> {
        self.collector
            .as_mut()
            .map(|c| c.processes(limit))
            .unwrap_or_default()
    }

    fn adapters(&mut self) -> Vec<String> {
        self.collector
            .as_ref()
            .map(|c| c.adapters().into_iter().map(|a| a.alias).collect())
            .unwrap_or_default()
    }

    fn set_adapter(&mut self, a: AdapterChoice) {
        if let Some(c) = &mut self.collector {
            c.set_adapter(a.clone());
        }
        self.adapter = a;
    }

    fn slept_s(&mut self) -> f64 {
        collect::slept_s()
    }

    fn network(&mut self) -> Network {
        Network {
            path_up: probe::path_up(),
            captive_portal: self.captive.load(Ordering::Relaxed),
            ..Network::default()
        }
    }
}
