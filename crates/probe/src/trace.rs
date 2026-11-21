use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

pub const MAX_TTL: u8 = 30;
const FULL_SWEEP_EVERY: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq)]
pub struct Hop {
    pub ttl: u8,
    pub addr: Option<IpAddr>,
    pub rtt_ms: Option<f32>,
    pub reached: bool,
}

/// Remembers how far away the target was so later ticks skip TTLs beyond it.
#[derive(Default)]
pub struct PathMemory {
    len: Option<u8>,
    last_full: Option<Instant>,
}

impl PathMemory {
    pub fn max_ttl(&self, now: Instant) -> u8 {
        match (self.len, self.last_full) {
            (Some(len), Some(at)) if now.duration_since(at) < FULL_SWEEP_EVERY => len,
            _ => MAX_TTL,
        }
    }

    /// Sorts `hops`, cuts them after the first one that reached the target, and learns the path length.
    pub fn record(&mut self, swept: u8, mut hops: Vec<Hop>, now: Instant) -> Vec<Hop> {
        hops.sort_by_key(|h| h.ttl);
        if let Some(i) = hops.iter().position(|h| h.reached) {
            hops.truncate(i + 1);
        }
        self.len = hops.last().filter(|h| h.reached).map(|h| h.ttl);
        if swept == MAX_TTL {
            self.last_full = Some(now);
        }
        hops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hop(ttl: u8, reached: bool) -> Hop {
        Hop {
            ttl,
            addr: None,
            rtt_ms: None,
            reached,
        }
    }

    #[test]
    fn learns_path_length_and_sweeps_fully_again_later() {
        let t0 = Instant::now();
        let mut m = PathMemory::default();
        assert_eq!(m.max_ttl(t0), MAX_TTL);
        let hops = (1..=MAX_TTL).rev().map(|t| hop(t, t >= 4)).collect();
        let kept = m.record(MAX_TTL, hops, t0);
        assert_eq!(kept.iter().map(|h| h.ttl).collect::<Vec<_>>(), [1, 2, 3, 4]);
        assert_eq!(m.max_ttl(t0 + Duration::from_secs(1)), 4);
        assert_eq!(m.max_ttl(t0 + FULL_SWEEP_EVERY), MAX_TTL);
    }

    #[test]
    fn path_grew_so_next_tick_sweeps_fully() {
        let t0 = Instant::now();
        let mut m = PathMemory::default();
        m.record(MAX_TTL, vec![hop(1, false), hop(2, true)], t0);
        assert_eq!(m.max_ttl(t0), 2);
        let kept = m.record(2, vec![hop(1, false), hop(2, false)], t0);
        assert_eq!(kept.len(), 2);
        assert_eq!(m.max_ttl(t0), MAX_TTL);
    }
}
