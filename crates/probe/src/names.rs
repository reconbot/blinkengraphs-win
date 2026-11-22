//! Reverse DNS for Hop addresses, like mtr (spec `rules/hop-trace.md`, "Names").

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Mutex, OnceLock},
    thread,
};

enum Entry {
    Pending,
    Done(Option<String>),
}

fn cache() -> &'static Mutex<HashMap<IpAddr, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<IpAddr, Entry>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The PTR name for `addr`, cached for the session. The first call starts a background lookup
/// and returns `None`, as does an address without a name; a lookup never blocks the caller.
pub fn name(addr: IpAddr) -> Option<String> {
    let mut c = cache().lock().unwrap();
    match c.get(&addr) {
        Some(Entry::Done(n)) => return n.clone(),
        Some(Entry::Pending) => return None,
        None => {}
    }
    c.insert(addr, Entry::Pending);
    drop(c);
    thread::spawn(move || {
        let n = lookup(addr);
        cache().lock().unwrap().insert(addr, Entry::Done(n));
    });
    None
}

#[cfg(windows)]
fn lookup(addr: IpAddr) -> Option<String> {
    crate::win::reverse_dns(addr)
}

#[cfg(not(windows))]
fn lookup(_: IpAddr) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_call_never_blocks_and_repeats_are_cached() {
        let a: IpAddr = "192.0.2.1".parse().unwrap();
        assert_eq!(name(a), None);
        assert!(cache().lock().unwrap().contains_key(&a));
    }
}
