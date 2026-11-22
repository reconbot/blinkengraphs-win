use collect::{AdapterChoice, Collector};
use model::{ProcessRow, Sample};

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
}

/// The OS adapter: performance counters through `collect`, network state through `probe`.
pub struct SystemSensors {
    adapter: AdapterChoice,
    collector: Option<Collector>,
}

impl SystemSensors {
    pub fn new(adapter: AdapterChoice) -> Self {
        Self {
            collector: Collector::new(adapter.clone()).ok(),
            adapter,
        }
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

    fn network(&mut self) -> Network {
        Network {
            path_up: probe::path_up(),
            ..Network::default()
        }
    }
}
