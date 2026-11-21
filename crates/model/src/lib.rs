#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Cpu,
    Memory,
    Network,
    Disk,
    Latency,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Cpu,
        Section::Memory,
        Section::Network,
        Section::Disk,
        Section::Latency,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Cpu => "CPU",
            Section::Memory => "MEM",
            Section::Network => "NET",
            Section::Disk => "HD",
            Section::Latency => "PING",
        }
    }
}
