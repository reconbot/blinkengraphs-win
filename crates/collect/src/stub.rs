use crate::{AdapterChoice, AdapterInfo};
use model::{ProcessRow, Sample};

pub struct Collector;

impl Collector {
    pub fn new(_adapter: AdapterChoice) -> std::io::Result<Self> {
        Ok(Self)
    }

    pub fn set_adapter(&mut self, _adapter: AdapterChoice) {}

    pub fn adapters(&self) -> Vec<AdapterInfo> {
        Vec::new()
    }

    pub fn sample(&mut self) -> Sample {
        Sample {
            at: model::now(),
            ..Sample::default()
        }
    }

    pub fn processes(&mut self, _limit: usize) -> Vec<ProcessRow> {
        Vec::new()
    }
}
