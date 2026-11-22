//! Where the Prober meets the network. The Ticker keeps the spec's rules (which Probes run,
//! absent versus lost, path memory); a Transport only sends and times.

use crate::net;
pub use crate::trace::Hop;
use model::{Link, Target};
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
    /// Whether `via` routes `to`. A Probe goes out on a Link only if it does (spec
    /// `rules/probe-stats.md`), so a split-tunnel VPN measures only Targets inside it.
    fn routes(&self, via: &Link, to: IpAddr) -> bool;
    /// The address for a Target; `Target::Addr` needs no lookup.
    fn lookup(&self, target: &Target) -> Option<IpAddr>;
    /// Each send below leaves from `via`'s interface, or wherever the OS routes it when `None`.
    fn tcp(&self, via: Option<&Link>, to: SocketAddr, timeout: Duration) -> Option<Duration>;
    fn udp(&self, via: Option<&Link>, to: SocketAddr, timeout: Duration) -> Option<Duration>;
    fn dns(
        &self,
        via: Option<&Link>,
        to: SocketAddr,
        timeout: Duration,
        name: &str,
    ) -> Option<Duration>;
    /// Every `(destination, ttl)` echo at once; `None` where nothing came back. The whole
    /// answer is `None` where ICMP can't be sent at all, so those Probes are skipped.
    fn echo(
        &self,
        via: Option<&Link>,
        requests: &[(IpAddr, u8)],
        timeout: Duration,
    ) -> Option<Vec<Option<Echo>>>;
    /// A TCP Hop Trace up to `max_ttl`, or `None` where it can't run (ADR 0006).
    fn tcp_trace(
        &self,
        via: Option<&Link>,
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

    fn routes(&self, via: &Link, to: IpAddr) -> bool {
        #[cfg(windows)]
        return crate::win::routes(via, to);
        #[cfg(not(windows))]
        {
            let _ = (via, to);
            true
        }
    }

    fn lookup(&self, target: &Target) -> Option<IpAddr> {
        crate::resolve::lookup(target)
    }

    fn tcp(&self, via: Option<&Link>, to: SocketAddr, timeout: Duration) -> Option<Duration> {
        net::tcp(via, to, timeout)
    }

    fn udp(&self, via: Option<&Link>, to: SocketAddr, timeout: Duration) -> Option<Duration> {
        net::udp(via, to, timeout)
    }

    fn dns(
        &self,
        via: Option<&Link>,
        to: SocketAddr,
        timeout: Duration,
        name: &str,
    ) -> Option<Duration> {
        net::dns(via, to, timeout, name)
    }

    fn echo(
        &self,
        via: Option<&Link>,
        requests: &[(IpAddr, u8)],
        timeout: Duration,
    ) -> Option<Vec<Option<Echo>>> {
        #[cfg(windows)]
        return self.icmp.as_ref().map(|icmp| {
            icmp.batch(via, requests, timeout)
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
            let _ = (via, requests, timeout);
            None
        }
    }

    fn tcp_trace(
        &self,
        via: Option<&Link>,
        to: Ipv4Addr,
        port: u16,
        max_ttl: u8,
        timeout: Duration,
    ) -> Option<Vec<Hop>> {
        #[cfg(windows)]
        return Some(crate::win::tcp_trace(via, to, port, max_ttl, timeout));
        #[cfg(not(windows))]
        {
            let _ = (via, to, port, max_ttl, timeout);
            None
        }
    }
}
