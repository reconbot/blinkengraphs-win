use model::Target;
use std::{
    collections::HashMap,
    net::{IpAddr, ToSocketAddrs},
    sync::Mutex,
    time::{Duration, Instant},
};

const REFRESH: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Resolver {
    cache: Mutex<HashMap<Target, (Option<IpAddr>, Instant)>>,
}

impl Resolver {
    /// Calls `changed` when the answer differs from the cached one, including the first answer.
    pub fn resolve(&self, target: &Target, changed: impl FnOnce(Option<IpAddr>)) -> Option<IpAddr> {
        if let Target::Addr(a) = target {
            return Some(*a);
        }
        if let Some(&(addr @ Some(_), at)) = self.cache.lock().unwrap().get(target)
            && at.elapsed() < REFRESH
        {
            return addr;
        }
        // Lookups can block for seconds; don't hold the lock across them.
        let addr = lookup(target);
        let prev = self
            .cache
            .lock()
            .unwrap()
            .insert(target.clone(), (addr, Instant::now()));
        if prev.map(|(a, _)| a) != Some(addr) {
            changed(addr);
        }
        addr
    }
}

fn lookup(target: &Target) -> Option<IpAddr> {
    match target {
        Target::Addr(a) => Some(*a),
        Target::Name(name) => {
            let addrs: Vec<IpAddr> = (name.as_str(), 0)
                .to_socket_addrs()
                .ok()?
                .map(|s| s.ip())
                .collect();
            // IcmpSendEcho2 and TCP hop tracing are IPv4-only.
            addrs
                .iter()
                .find(|a| a.is_ipv4())
                .or(addrs.first())
                .copied()
        }
        #[cfg(windows)]
        Target::Gateway => crate::win::gateway(),
        #[cfg(windows)]
        Target::SystemResolver => crate::win::system_resolver(),
        #[cfg(not(windows))]
        Target::Gateway | Target::SystemResolver => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_first_answer_then_only_changes() {
        let r = Resolver::default();
        let host = Target::Name("localhost".into());
        let mut seen = vec![];
        let first = r.resolve(&host, |a| seen.push(a));
        assert!(first.is_some_and(|a| a.is_loopback()));
        r.resolve(&host, |a| seen.push(a));
        assert_eq!(seen, vec![first]);
    }

    #[test]
    fn literal_addresses_need_no_lookup() {
        let r = Resolver::default();
        let a: IpAddr = "192.0.2.1".parse().unwrap();
        assert_eq!(r.resolve(&Target::Addr(a), |_| panic!()), Some(a));
    }
}
