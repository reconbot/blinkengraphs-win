use collect::{AdapterChoice, Collector};
use model::{ProcessRow, Sample};
use std::{
    sync::{
        Arc, Mutex,
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
    /// The Links up now, each to be measured on its own (spec `CONTEXT.md`, Link).
    fn links(&mut self) -> Vec<model::Link>;
    /// The latest Public Address reading, IPv4 then IPv6; `None` before the first lookup.
    fn public_address(&mut self) -> Option<[probe::public::Reading; 2]>;
    /// Looks the Public Address up again soon, as after a Link change.
    fn refresh_public(&mut self);
    /// The Links' adapters, by NET_LUID, whose throughput each Sample also reports.
    fn set_links(&mut self, luids: Vec<u64>);
}

/// The OS adapter: performance counters through `collect`, network state through `probe`.
pub struct SystemSensors {
    adapter: AdapterChoice,
    collector: Option<Collector>,
    captive: Arc<AtomicBool>,
    public: Arc<Mutex<Option<[probe::public::Reading; 2]>>>,
    refresh: Arc<AtomicBool>,
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
        let public = Arc::new(Mutex::new(None));
        let refresh = Arc::new(AtomicBool::new(false));
        let (p, r) = (public.clone(), refresh.clone());
        thread::Builder::new()
            .name("public address".into())
            .spawn(move || watch_public(&p, &r))
            .expect("spawn public address lookups");
        Self {
            collector: Collector::new(adapter.clone()).ok(),
            adapter,
            captive,
            public,
            refresh,
        }
    }
}

/// Looks the Public Address up on start, every `refresh_s`, and soon after `refresh` is set
/// (spec `rules/public-address.md`, Refresh).
fn watch_public(out: &Mutex<Option<[probe::public::Reading; 2]>>, refresh: &AtomicBool) {
    let d = config::defaults::get().public_address;
    let t = Duration::from_secs(5);
    loop {
        if probe::path_up() {
            let look =
                |v6: bool, url: &str| probe::public::lookup(&d.dns_server, &d.dns_name, url, v6, t);
            let r = [
                look(false, &d.https_url_ipv4),
                look(true, &d.https_url_ipv6),
            ];
            *out.lock().unwrap() = Some(r);
        }
        for _ in 0..d.refresh_s {
            if refresh.swap(false, Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_secs(1));
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

    fn links(&mut self) -> Vec<model::Link> {
        probe::links()
    }

    fn public_address(&mut self) -> Option<[probe::public::Reading; 2]> {
        self.public.lock().unwrap().clone()
    }

    fn refresh_public(&mut self) {
        self.refresh.store(true, Ordering::Relaxed);
    }

    fn set_links(&mut self, luids: Vec<u64>) {
        if let Some(c) = &mut self.collector {
            c.set_links(luids);
        }
    }

    fn network(&mut self) -> Network {
        Network {
            path_up: probe::path_up(),
            captive_portal: self.captive.load(Ordering::Relaxed),
            ..Network::default()
        }
    }
}
