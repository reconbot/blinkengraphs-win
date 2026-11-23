//! Reading the Public Address lookups into one state per IP version (spec `rules/public-address.md`).

use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket},
    time::Duration,
};

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

/// Both lookups for one IP version, read into its state (spec `rules/public-address.md`).
pub fn lookup(
    dns_server: &str,
    dns_name: &str,
    https_url: &str,
    v6: bool,
    timeout: Duration,
) -> Reading {
    let dns = dns_txt(dns_server, dns_name, v6, timeout);
    let https = https_body(https_url, timeout);
    read(v6, dns.as_deref(), https.as_deref())
}

/// The TXT strings for `name`, asked of `server` over this IP version.
fn dns_txt(server: &str, name: &str, v6: bool, timeout: Duration) -> Option<Vec<String>> {
    let addr = (server, 53)
        .to_socket_addrs()
        .ok()?
        .find(|a| a.is_ipv6() == v6)?;
    txt_at(addr, name, timeout)
}

fn txt_at(addr: SocketAddr, name: &str, timeout: Duration) -> Option<Vec<String>> {
    let local: SocketAddr = if addr.is_ipv6() {
        "[::]:0".parse().unwrap()
    } else {
        "0.0.0.0:0".parse().unwrap()
    };
    let socket = UdpSocket::bind(local).ok()?;
    socket.connect(addr).ok()?;
    socket.set_read_timeout(Some(timeout)).ok()?;
    let id = (std::process::id() as u16) ^ 0x5a5a;
    socket
        .send(&crate::dns::encode_query_type(
            id,
            name,
            crate::dns::TYPE_TXT,
        ))
        .ok()?;
    let mut buf = [0u8; 1500];
    let n = socket.recv(&mut buf).ok()?;
    crate::dns::txt_answers(&buf[..n], id)
}

/// The body of `GET url` over HTTPS, through WinHTTP so TLS and proxies are Windows' own.
/// Off Windows there is no HTTPS lookup, and the DNS one stands alone.
fn https_body(url: &str, timeout: Duration) -> Option<String> {
    #[cfg(windows)]
    return crate::win::https_get(url, timeout);
    #[cfg(not(windows))]
    {
        let _ = (url, timeout);
        None
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    #[test]
    fn asks_a_server_for_txt_and_reads_the_answer() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            let mut r = buf[..n].to_vec();
            r[2] |= 0x80;
            r[7] = 1;
            let txt = b"remote_ip: 198.51.100.7";
            r.extend_from_slice(&[0xc0, 12, 0, 16, 0, 1, 0, 0, 0, 0]);
            r.extend_from_slice(&((txt.len() + 1) as u16).to_be_bytes());
            r.push(txt.len() as u8);
            r.extend_from_slice(txt);
            server.send_to(&r, from).unwrap();
        });
        let got = txt_at(addr, "whoami.cloudflare.com", Duration::from_secs(2)).unwrap();
        assert_eq!(
            read(false, Some(&got), None),
            Reading::Single {
                address: "198.51.100.7".parse().unwrap(),
                source: Source::Dns,
                asn: None,
            }
        );
    }
}
