//! Usage: linkcheck [--label TEXT] [--raw] [--resume [N]]
//!
//! Answers on a real machine what ADR 0019, ADR 0020 and research 07 leave unverified, and
//! prints a Markdown report to stdout:
//! - which adapters Windows lists, how the app classes them, and which Link it would follow;
//! - `GetBestRoute2` constrained to each Link, for each Target;
//! - each Probe pinned to each Link and unpinned, with the Win32 error when it fails, so a
//!   firewall block (a VPN's kill switch) can be told from a loss;
//! - the Tailscale state and DNS policy, when Tailscale is installed.
//!
//! `--resume` waits for the machine to sleep N times (default 3) and logs, per wake, how far
//! each clock moved and how long each Link took to answer again: the settle-time data.
//!
//! The report hides public and Tailscale addresses and Gateway MACs' device bytes; `--raw`
//! keeps them.

#[cfg(windows)]
mod win;

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

pub struct Args {
    pub label: Option<String>,
    pub raw: bool,
    pub resume: Option<u32>,
}

fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut out = Args {
        label: None,
        raw: false,
        resume: None,
    };
    let mut it = args.into_iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--label" => out.label = Some(it.next().ok_or("--label needs a value")?),
            "--raw" => out.raw = true,
            "--resume" => {
                let n = it.next_if(|n| n.parse::<u32>().is_ok());
                out.resume = Some(n.map_or(3, |n| n.parse().unwrap()));
            }
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(out)
}

const USAGE: &str = "usage: linkcheck [--label TEXT] [--raw] [--resume [N]]";

/// One send's result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Reply(Duration),
    /// Nothing came back in time.
    Timeout,
    /// The OS refused or failed the send; the Win32 or Winsock code.
    Error(u32),
    /// Not attempted: no address of this IP version on the Link, or no Target.
    Skipped,
}

/// Several tries of one send, as `3/3 12.4 ms`, `0/3 timeout`, or `0/3 10013 WSAEACCES`.
pub fn summarize(tries: &[Outcome]) -> String {
    if tries.iter().all(|o| *o == Outcome::Skipped) {
        return "–".into();
    }
    let mut rtts: Vec<f64> = tries
        .iter()
        .filter_map(|o| match o {
            Outcome::Reply(d) => Some(d.as_secs_f64() * 1000.0),
            _ => None,
        })
        .collect();
    rtts.sort_by(f64::total_cmp);
    let mut s = format!("{}/{}", rtts.len(), tries.len());
    if !rtts.is_empty() {
        s += &format!(" {:.1} ms", rtts[rtts.len() / 2]);
    }
    let mut errors: Vec<u32> = tries
        .iter()
        .filter_map(|o| match o {
            Outcome::Error(c) => Some(*c),
            _ => None,
        })
        .collect();
    errors.dedup();
    for c in errors {
        s += &format!(" err {c} {}", error_name(c));
    }
    if tries.contains(&Outcome::Timeout) {
        s += " timeout";
    }
    s
}

/// Names for the codes a blocked, unrouted or unanswered send returns.
/// https://learn.microsoft.com/windows/win32/winsock/windows-sockets-error-codes-2
/// https://learn.microsoft.com/windows/win32/api/ipexport/ns-ipexport-icmp_echo_reply
pub fn error_name(code: u32) -> &'static str {
    match code {
        5 => "ERROR_ACCESS_DENIED",
        50 => "ERROR_NOT_SUPPORTED",
        87 => "ERROR_INVALID_PARAMETER",
        1168 => "ERROR_NOT_FOUND",
        1231 => "ERROR_NETWORK_UNREACHABLE",
        1232 => "ERROR_HOST_UNREACHABLE",
        1236 => "ERROR_CONNECTION_ABORTED",
        10013 => "WSAEACCES",
        10022 => "WSAEINVAL",
        10049 => "WSAEADDRNOTAVAIL",
        10050 => "WSAENETDOWN",
        10051 => "WSAENETUNREACH",
        10054 => "WSAECONNRESET",
        10060 => "WSAETIMEDOUT",
        10065 => "WSAEHOSTUNREACH",
        11002 => "IP_DEST_NET_UNREACHABLE",
        11003 => "IP_DEST_HOST_UNREACHABLE",
        11004 => "IP_DEST_PROT_UNREACHABLE",
        11005 => "IP_DEST_PORT_UNREACHABLE",
        11010 => "IP_REQ_TIMED_OUT",
        11013 => "IP_TTL_EXPIRED_TRANSIT",
        11050 => "IP_GENERAL_FAILURE",
        _ => "",
    }
}

/// Hides what identifies the owner: public addresses keep their first half, Tailscale
/// addresses their range. Private, link-local and the well-known Targets stay.
pub fn redact_ip(ip: IpAddr, raw: bool) -> String {
    if raw || known(ip) {
        return ip.to_string();
    }
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            if a.is_private() || a.is_loopback() || a.is_link_local() {
                a.to_string()
            } else if o[0] == 100 && (o[1] & 0xc0) == 64 {
                "100.x.x.x (CGNAT/Tailscale)".into()
            } else {
                format!("{}.{}.x.x", o[0], o[1])
            }
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            if (s[0] & 0xffc0) == 0xfe80 || a.is_loopback() || (s[0] & 0xfe00) == 0xfc00 {
                // Link-local and ULA say nothing outside the LAN, but the interface id can.
                format!("{:x}:{:x}::x", s[0], s[1])
            } else if s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0 {
                "fd7a:115c:a1e0::x (Tailscale)".into()
            } else {
                format!("{:x}:{:x}::x", s[0], s[1])
            }
        }
    }
}

fn known(ip: IpAddr) -> bool {
    const KNOWN: [IpAddr; 7] = [
        IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
        IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)),
        IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
        IpAddr::V4(Ipv4Addr::new(100, 100, 100, 100)),
        IpAddr::V6(Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111)),
        IpAddr::V6(Ipv6Addr::new(0x2620, 0xfe, 0, 0, 0, 0, 0, 0xfe)),
        IpAddr::V6(Ipv6Addr::new(0xfd7a, 0x115c, 0xa1e0, 0, 0, 0, 0, 0x53)),
    ];
    KNOWN.contains(&ip) || ip.is_unspecified()
}

/// A MAC keeps its vendor prefix (OUI) and hides the device bytes.
pub fn redact_mac(mac: [u8; 6], raw: bool) -> String {
    let hex: Vec<String> = mac.iter().map(|b| format!("{b:02x}")).collect();
    if raw {
        hex.join("-")
    } else {
        format!("{}-xx-xx-xx", hex[..3].join("-"))
    }
}

/// A Markdown table; `|` in cells is escaped.
pub fn table(head: &[&str], rows: &[Vec<String>]) -> String {
    let cell = |s: &str| s.replace('|', "\\|");
    let mut out = format!("| {} |\n", head.join(" | "));
    out += &format!("|{}\n", "---|".repeat(head.len()));
    for r in rows {
        let r: Vec<String> = r.iter().map(|c| cell(c)).collect();
        out += &format!("| {} |\n", r.join(" | "));
    }
    out
}

fn main() {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    #[cfg(windows)]
    {
        if let Some(n) = args.resume {
            win::resume(&args, n);
        } else {
            print!("{}", win::snapshot(&args));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        eprintln!("linkcheck measures Windows networking and runs only on Windows");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Result<Args, String> {
        parse(s.split_whitespace().map(String::from))
    }

    #[test]
    fn parses_flags() {
        let a = args("--label wifi --raw --resume").unwrap();
        assert_eq!(a.label.as_deref(), Some("wifi"));
        assert!(a.raw);
        assert_eq!(a.resume, Some(3));
        assert_eq!(args("--resume 5").unwrap().resume, Some(5));
        assert!(args("--nope").is_err());
    }

    #[test]
    fn summarizes_tries() {
        let ms = |m| Outcome::Reply(Duration::from_millis(m));
        assert_eq!(summarize(&[ms(10), ms(30), ms(20)]), "3/3 20.0 ms");
        assert_eq!(
            summarize(&[
                Outcome::Error(10013),
                Outcome::Error(10013),
                Outcome::Timeout
            ]),
            "0/3 err 10013 WSAEACCES timeout"
        );
        assert_eq!(summarize(&[Outcome::Skipped]), "–");
    }

    #[test]
    fn redacts_what_identifies_the_owner() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(redact_ip(ip("192.168.1.1"), false), "192.168.1.1");
        assert_eq!(redact_ip(ip("1.1.1.1"), false), "1.1.1.1");
        assert_eq!(redact_ip(ip("100.100.100.100"), false), "100.100.100.100");
        assert_eq!(redact_ip(ip("203.0.113.9"), false), "203.0.x.x");
        assert_eq!(
            redact_ip(ip("100.101.102.103"), false),
            "100.x.x.x (CGNAT/Tailscale)"
        );
        assert_eq!(redact_ip(ip("2001:db8:1:2::5"), false), "2001:db8::x");
        assert_eq!(redact_ip(ip("203.0.113.9"), true), "203.0.113.9");
        assert_eq!(
            redact_mac([0, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e], false),
            "00-1a-2b-xx-xx-xx"
        );
    }

    #[test]
    fn tables_escape_pipes() {
        let t = table(&["a", "b"], &[vec!["x|y".into(), "z".into()]]);
        assert_eq!(t, "| a | b |\n|---|---|\n| x\\|y | z |\n");
    }
}
