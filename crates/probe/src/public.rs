//! Reading the Public Address lookups into one state per IP version (spec `rules/public-address.md`).

use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Dns,
    Https,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reading {
    /// Neither lookup answered.
    None,
    /// An IPv6 lookup answered with an IPv4 address: an IPv6-only network with NAT64.
    Nat64(IpAddr),
    Agree {
        address: IpAddr,
        asn: Option<String>,
    },
    /// DNS and HTTPS leave by different paths.
    Differ {
        dns: IpAddr,
        https: IpAddr,
        asn: Option<String>,
    },
    Single {
        address: IpAddr,
        source: Source,
        asn: Option<String>,
    },
}

/// `dns_txt` is the TXT strings of `whoami.cloudflare.com` and `https_body` the checkip body,
/// each `None` when that lookup failed. An answer that isn't an address counts as no answer.
pub fn read(v6: bool, dns_txt: Option<&[String]>, https_body: Option<&str>) -> Reading {
    let field = |key: &str| {
        dns_txt?
            .iter()
            .find_map(|t| t.strip_prefix(key)?.strip_prefix(':').map(str::trim))
            .map(str::to_string)
    };
    let dns = field("remote_ip").and_then(|a| a.parse::<IpAddr>().ok());
    let asn = field("asn");
    let https = https_body.and_then(|b| b.trim().parse::<IpAddr>().ok());
    if v6 && let Some(a) = [dns, https].into_iter().flatten().find(IpAddr::is_ipv4) {
        return Reading::Nat64(a);
    }
    match (dns, https) {
        (None, None) => Reading::None,
        (Some(d), Some(h)) if d == h => Reading::Agree { address: d, asn },
        (Some(dns), Some(https)) => Reading::Differ { dns, https, asn },
        (Some(address), None) => Reading::Single {
            address,
            source: Source::Dns,
            asn,
        },
        (None, Some(address)) => Reading::Single {
            address,
            source: Source::Https,
            asn,
        },
    }
}
