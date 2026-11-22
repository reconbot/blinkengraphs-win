//! Where the Prober meets the network. The Ticker keeps the spec's rules (which Probes run,
//! absent versus lost, path memory); a Transport only sends and times.

use crate::net;
pub use crate::trace::Hop;
use model::Target;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

/// One ICMP echo's answer: the target's reply, or a router's Time Exceeded on a Hop Trace.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Echo {
    pub from: IpAddr,
    pub rtt: Duration,
    /// The target itself replied, rather than a router on the way.
    pub reached: bool,
}

pub trait Transport: Send + Sync + 'static {
    /// Whether the OS has a route for this IP version.
    fn family_up(&self, v6: bool) -> bool;
    /// The address for a Target; `Target::Addr` needs no lookup.
    fn lookup(&self, target: &Target) -> Option<IpAddr>;
    fn tcp(&self, to: SocketAddr, timeout: Duration) -> Option<Duration>;
    fn udp(&self, to: SocketAddr, timeout: Duration) -> Option<Duration>;
    fn dns(&self, to: SocketAddr, timeout: Duration, name: &str) -> Option<Duration>;
    /// Every `(destination, ttl)` echo at once; `None` where nothing came back. The whole
    /// answer is `None` where ICMP can't be sent at all, so those Probes are skipped.
    fn echo(&self, requests: &[(IpAddr, u8)], timeout: Duration) -> Option<Vec<Option<Echo>>>;
    /// A TCP Hop Trace up to `max_ttl`, or `None` where it can't run (ADR 0006).
    fn tcp_trace(
        &self,
        to: Ipv4Addr,
        port: u16,
        max_ttl: u8,
        timeout: Duration,
    ) -> Option<Vec<Hop>>;
}

/// The OS adapter: IP Helper and sockets on Windows; sockets alone elsewhere, without ICMP.
pub struct SystemTransport {
    #[cfg(windows)]
    icmp: Option<crate::win::Icmp>,
}

// Derivable only where there is no ICMP handle to open.
#[cfg_attr(not(windows), allow(clippy::derivable_impls))]
impl Default for SystemTransport {
    fn default() -> Self {
        Self {
            #[cfg(windows)]
            icmp: crate::win::Icmp::new(),
        }
    }
}

impl Transport for SystemTransport {
    fn family_up(&self, v6: bool) -> bool {
        crate::family_up(v6)
    }

    fn lookup(&self, target: &Target) -> Option<IpAddr> {
        crate::resolve::lookup(target)
    }

    fn tcp(&self, to: SocketAddr, timeout: Duration) -> Option<Duration> {
        net::tcp(to, timeout)
    }

    fn udp(&self, to: SocketAddr, timeout: Duration) -> Option<Duration> {
        net::udp(to, timeout)
    }

    fn dns(&self, to: SocketAddr, timeout: Duration, name: &str) -> Option<Duration> {
        net::dns(to, timeout, name)
    }

    fn echo(&self, requests: &[(IpAddr, u8)], timeout: Duration) -> Option<Vec<Option<Echo>>> {
        #[cfg(windows)]
        return self.icmp.as_ref().map(|icmp| {
            icmp.batch(requests, timeout)
                .into_iter()
                .map(|r| {
                    r.map(|r| Echo {
                        from: r.from,
                        rtt: r.rtt,
                        reached: r.status == crate::win::IP_SUCCESS,
                    })
                })
                .collect()
        });
        #[cfg(not(windows))]
        {
            let _ = (requests, timeout);
            None
        }
    }

    fn tcp_trace(
        &self,
        to: Ipv4Addr,
        port: u16,
        max_ttl: u8,
        timeout: Duration,
    ) -> Option<Vec<Hop>> {
        #[cfg(windows)]
        return Some(crate::win::tcp_trace(to, port, max_ttl, timeout));
        #[cfg(not(windows))]
        {
            let _ = (to, port, max_ttl, timeout);
            None
        }
    }
}
