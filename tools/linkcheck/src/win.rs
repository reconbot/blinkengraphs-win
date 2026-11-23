//! The Win32 side: adapters, routes, pinned sends with their error codes, clocks.

use crate::{Args, Outcome, redact_ip, redact_mac, summarize, table};
use model::{Link, LinkKind};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::{
    collections::BTreeMap,
    io::ErrorKind,
    mem::{size_of, zeroed},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    os::windows::io::AsRawSocket,
    process::Command,
    ptr::{null, null_mut},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{ERROR_BUFFER_OVERFLOW, GetLastError, NO_ERROR},
    NetworkManagement::IpHelper::{
        GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST,
        GetAdaptersAddresses, GetBestRoute2, GetIpInterfaceEntry, ICMP_ECHO_REPLY,
        ICMPV6_ECHO_REPLY_LH, IP_ADAPTER_ADDRESSES_LH, IP_REQ_TIMED_OUT, IP_SUCCESS,
        Icmp6CreateFile, Icmp6SendEcho2, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho2Ex,
        MIB_IPFORWARD_ROW2, MIB_IPINTERFACE_ROW,
    },
    NetworkManagement::Ndis::{IfOperStatusUp, NET_LUID_LH},
    Networking::WinSock::{
        AF_INET, AF_INET6, AF_UNSPEC, IN_ADDR, IN_ADDR_0, IN6_ADDR, IN6_ADDR_0, IP_UNICAST_IF,
        IPPROTO_IP, IPPROTO_IPV6, IPV6_UNICAST_IF, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6,
        SOCKADDR_INET, SOCKET, setsockopt,
    },
    System::WindowsProgramming::{QueryInterruptTime, QueryUnbiasedInterruptTime},
};

const TRIES: usize = 3;
const TIMEOUT: Duration = Duration::from_secs(1);
const CF4: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
const CF6: Ipv6Addr = Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111);
const MAGIC_DNS: Ipv4Addr = Ipv4Addr::new(100, 100, 100, 100);

/// An adapter as GetAdaptersAddresses lists it, up or not.
struct Adapter {
    luid: u64,
    alias: String,
    description: String,
    if_type: u32,
    up: bool,
    gateways: Vec<IpAddr>,
    dns: Vec<IpAddr>,
}

/// https://learn.microsoft.com/windows/win32/api/iphlpapi/nf-iphlpapi-getadaptersaddresses
fn adapters() -> Vec<Adapter> {
    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let mut size: u32 = 32 * 1024;
    let mut buf: Vec<u64> = Vec::new();
    for _ in 0..4 {
        buf = vec![0; (size as usize).div_ceil(8)];
        let rc = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                flags,
                null(),
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        match rc {
            NO_ERROR => break,
            ERROR_BUFFER_OVERFLOW => continue,
            _ => return vec![],
        }
    }
    let mut out = Vec::new();
    let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while let Some(a) = unsafe { p.as_ref() } {
        p = a.Next;
        let mut gateways = Vec::new();
        let mut g = a.FirstGatewayAddress;
        while let Some(e) = unsafe { g.as_ref() } {
            gateways.extend(socket_ip(e.Address.lpSockaddr));
            g = e.Next;
        }
        let mut dns = Vec::new();
        let mut d = a.FirstDnsServerAddress;
        while let Some(e) = unsafe { d.as_ref() } {
            dns.extend(socket_ip(e.Address.lpSockaddr));
            d = e.Next;
        }
        out.push(Adapter {
            luid: unsafe { a.Luid.Value },
            alias: wide(a.FriendlyName),
            description: wide(a.Description),
            if_type: a.IfType,
            up: a.OperStatus == IfOperStatusUp,
            gateways,
            dns,
        });
    }
    out
}

/// Metric, whether it is automatic, and the weak host send flag, for one IP version.
/// https://learn.microsoft.com/windows/win32/api/netioapi/nf-netioapi-getipinterfaceentry
fn interface_row(luid: u64, v6: bool) -> Option<MIB_IPINTERFACE_ROW> {
    let mut row: MIB_IPINTERFACE_ROW = unsafe { zeroed() };
    row.Family = if v6 { AF_INET6 } else { AF_INET };
    row.InterfaceLuid = NET_LUID_LH { Value: luid };
    (unsafe { GetIpInterfaceEntry(&mut row) } == NO_ERROR).then_some(row)
}

/// The interface and next hop of the best route to `to`, constrained to `luid` when given.
/// https://learn.microsoft.com/windows/win32/api/netioapi/nf-netioapi-getbestroute2
fn route(luid: Option<u64>, to: IpAddr) -> Result<(u64, Option<IpAddr>), u32> {
    let dest = sockaddr_inet(to);
    let mut row: MIB_IPFORWARD_ROW2 = unsafe { zeroed() };
    let mut source = SOCKADDR_INET::default();
    let luid = luid.map(|v| NET_LUID_LH { Value: v });
    let luid_ptr = luid.as_ref().map_or(null(), |l| l as *const NET_LUID_LH);
    let rc = unsafe { GetBestRoute2(luid_ptr, 0, null(), &dest, 0, &mut row, &mut source) };
    if rc != NO_ERROR {
        return Err(rc);
    }
    let hop = inet_ip(&row.NextHop).filter(|a| !a.is_unspecified());
    Ok((unsafe { row.InterfaceLuid.Value }, hop))
}

/// Pins a socket to `link`'s interface, as the app's Prober does (ADR 0020).
fn unicast_if(raw: SOCKET, v6: bool, link: &Link) -> Result<(), u32> {
    let (level, name, index) = if v6 {
        (IPPROTO_IPV6, IPV6_UNICAST_IF, link.index_v6)
    } else {
        (IPPROTO_IP, IP_UNICAST_IF, link.index_v4.to_be())
    };
    if index == 0 {
        return Err(10049);
    }
    let rc = unsafe { setsockopt(raw, level, name, (&index as *const u32).cast(), 4) };
    if rc == 0 { Ok(()) } else { Err(os_error()) }
}

fn os_error() -> u32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(-1) as u32
}

fn io_outcome(e: &std::io::Error) -> Outcome {
    match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => Outcome::Timeout,
        _ => Outcome::Error(e.raw_os_error().unwrap_or(-1) as u32),
    }
}

/// One echo, synchronously, from the Link's address as the app sends it.
/// https://learn.microsoft.com/windows/win32/api/icmpapi/nf-icmpapi-icmpsendecho2ex
fn icmp(via: Option<&Link>, to: IpAddr) -> Outcome {
    let payload = [0u8; 32];
    let mut reply = [0u64; 128];
    let size = size_of::<[u64; 128]>() as u32;
    let ms = TIMEOUT.as_millis() as u32;
    let start = Instant::now();
    let (n, status) = match to {
        IpAddr::V4(dst) => {
            let src = match via {
                Some(l) => match l.source_v4 {
                    Some(a) => u32::from_ne_bytes(a.octets()),
                    None => return Outcome::Skipped,
                },
                None => 0,
            };
            let h = unsafe { IcmpCreateFile() };
            let n = unsafe {
                IcmpSendEcho2Ex(
                    h,
                    null_mut(),
                    None,
                    null(),
                    src,
                    u32::from_ne_bytes(dst.octets()),
                    payload.as_ptr().cast(),
                    payload.len() as u16,
                    null(),
                    reply.as_mut_ptr().cast(),
                    size,
                    ms,
                )
            };
            let err = unsafe { GetLastError() };
            unsafe { IcmpCloseHandle(h) };
            let r: ICMP_ECHO_REPLY = unsafe { reply.as_ptr().cast::<ICMP_ECHO_REPLY>().read() };
            (n, if n == 0 { err } else { r.Status })
        }
        IpAddr::V6(dst) => {
            let src = match via {
                Some(l) => match l.source_v6 {
                    Some(a) => a,
                    None => return Outcome::Skipped,
                },
                None => Ipv6Addr::UNSPECIFIED,
            };
            let (src, dst) = (sockaddr_in6(src), sockaddr_in6(dst));
            let h = unsafe { Icmp6CreateFile() };
            let n = unsafe {
                Icmp6SendEcho2(
                    h,
                    null_mut(),
                    None,
                    null(),
                    &src,
                    &dst,
                    payload.as_ptr().cast(),
                    payload.len() as u16,
                    null(),
                    reply.as_mut_ptr().cast(),
                    size,
                    ms,
                )
            };
            let err = unsafe { GetLastError() };
            unsafe { IcmpCloseHandle(h) };
            let r: ICMPV6_ECHO_REPLY_LH = unsafe {
                reply
                    .as_ptr()
                    .cast::<ICMPV6_ECHO_REPLY_LH>()
                    .read_unaligned()
            };
            (n, if n == 0 { err } else { r.Status })
        }
    };
    match (n, status) {
        (n, IP_SUCCESS) if n > 0 => Outcome::Reply(start.elapsed()),
        (_, IP_REQ_TIMED_OUT) => Outcome::Timeout,
        (_, s) => Outcome::Error(s),
    }
}

/// A TCP handshake; a refusal is a reply, as in the app.
fn tcp(via: Option<&Link>, to: SocketAddr) -> Outcome {
    let s = match Socket::new(Domain::for_address(to), Type::STREAM, Some(Protocol::TCP)) {
        Ok(s) => s,
        Err(e) => return io_outcome(&e),
    };
    if let Some(l) = via
        && let Err(c) = unicast_if(s.as_raw_socket() as SOCKET, to.is_ipv6(), l)
    {
        return Outcome::Error(c);
    }
    let start = Instant::now();
    match s.connect_timeout(&to.into(), TIMEOUT) {
        Ok(()) => Outcome::Reply(start.elapsed()),
        Err(e) if e.kind() == ErrorKind::ConnectionRefused => Outcome::Reply(start.elapsed()),
        Err(e) => io_outcome(&e),
    }
}

/// An `A` query for example.com over UDP.
fn dns(via: Option<&Link>, to: SocketAddr) -> Outcome {
    let s = match Socket::new(Domain::for_address(to), Type::DGRAM, Some(Protocol::UDP)) {
        Ok(s) => s,
        Err(e) => return io_outcome(&e),
    };
    if let Some(l) = via
        && let Err(c) = unicast_if(s.as_raw_socket() as SOCKET, to.is_ipv6(), l)
    {
        return Outcome::Error(c);
    }
    let any: SocketAddr = if to.is_ipv6() {
        (Ipv6Addr::UNSPECIFIED, 0).into()
    } else {
        (Ipv4Addr::UNSPECIFIED, 0).into()
    };
    if let Err(e) = s.bind(&any.into()) {
        return io_outcome(&e);
    }
    let _ = s.set_read_timeout(Some(TIMEOUT));
    let id = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
        & 0xffff) as u16;
    let mut q = id.to_be_bytes().to_vec();
    q.extend_from_slice(&[1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in ["example", "com"] {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 1, 0, 1]);
    let start = Instant::now();
    if let Err(e) = s.send_to(&q, &SockAddr::from(to)) {
        return io_outcome(&e);
    }
    let mut buf = [std::mem::MaybeUninit::<u8>::uninit(); 512];
    loop {
        match s.recv(&mut buf) {
            Ok(n) if n >= 2 => {
                let got = unsafe { [buf[0].assume_init(), buf[1].assume_init()] };
                if u16::from_be_bytes(got) == id {
                    return Outcome::Reply(start.elapsed());
                }
            }
            Ok(_) => {}
            Err(e) => return io_outcome(&e),
        }
        if start.elapsed() > TIMEOUT {
            return Outcome::Timeout;
        }
    }
}

/// Interrupt time counts sleep; the unbiased one doesn't. Both in 100 ns units.
/// https://learn.microsoft.com/windows/win32/api/realtimeapiset/nf-realtimeapiset-queryunbiasedinterrupttime
fn clocks() -> (u64, u64) {
    let (mut biased, mut unbiased) = (0u64, 0u64);
    unsafe {
        QueryUnbiasedInterruptTime(&mut unbiased);
        QueryInterruptTime(&mut biased);
    }
    (biased, unbiased)
}

fn slept_s() -> f64 {
    let (b, u) = clocks();
    b.saturating_sub(u) as f64 / 1e7
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Icmp,
    Tcp443,
    Dns,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Icmp => "ICMP",
            Kind::Tcp443 => "TCP 443",
            Kind::Dns => "DNS",
        }
    }

    fn send(self, via: Option<&Link>, to: IpAddr) -> Outcome {
        match self {
            Kind::Icmp => icmp(via, to),
            Kind::Tcp443 => tcp(via, (to, 443).into()),
            Kind::Dns => dns(via, (to, 53).into()),
        }
    }
}

/// A row of the send and route matrices: a named Target and what to send it.
struct Target {
    name: String,
    ip: IpAddr,
    kinds: Vec<Kind>,
}

fn targets(links: &[Link], resolver: Option<IpAddr>, tailscale: bool, raw: bool) -> Vec<Target> {
    let mut out: Vec<Target> = links
        .iter()
        .filter_map(|l| {
            let gw = l.id.gateway?;
            Some(Target {
                name: format!("Gateway of {} ({})", l.alias, redact_ip(gw, raw)),
                ip: gw,
                kinds: vec![Kind::Icmp, Kind::Dns],
            })
        })
        .collect();
    out.push(Target {
        name: "1.1.1.1".into(),
        ip: CF4.into(),
        kinds: vec![Kind::Icmp, Kind::Tcp443, Kind::Dns],
    });
    out.push(Target {
        name: CF6.to_string(),
        ip: CF6.into(),
        kinds: vec![Kind::Icmp, Kind::Tcp443, Kind::Dns],
    });
    if let Some(r) = resolver {
        out.push(Target {
            name: format!("system-dns ({})", redact_ip(r, raw)),
            ip: r,
            kinds: vec![Kind::Dns],
        });
    }
    if tailscale && resolver != Some(MAGIC_DNS.into()) {
        out.push(Target {
            name: "MagicDNS 100.100.100.100".into(),
            ip: MAGIC_DNS.into(),
            kinds: vec![Kind::Icmp, Kind::Dns],
        });
    }
    out
}

fn kind_name(k: LinkKind) -> &'static str {
    match k {
        LinkKind::Ethernet => "Ethernet",
        LinkKind::WiFi => "Wi-Fi",
        LinkKind::Vpn => "VPN",
        LinkKind::Other => "Other",
    }
}

/// The adapter list, routes and sends, as Markdown.
pub fn snapshot(args: &Args) -> String {
    let raw = args.raw;
    eprintln!("linkcheck: reading adapters…");
    let all = adapters();
    let links = probe::links();
    let alias = |luid: u64| {
        all.iter()
            .find(|a| a.luid == luid)
            .map_or_else(|| format!("{luid:016x}"), |a| a.alias.clone())
    };
    let tailscale_adapter = all
        .iter()
        .any(|a| a.up && a.description.to_lowercase().contains("tailscale"));

    let mut out = format!(
        "# linkcheck{}\n\n{}. linkcheck {}. Slept since boot: {:.0} s.\n\n",
        args.label
            .as_ref()
            .map_or(String::new(), |l| format!(": {l}")),
        windows_version(),
        env!("CARGO_PKG_VERSION"),
        slept_s(),
    );

    out += &tailscale(raw);
    out += &nrpt(raw);

    out += "## Adapters\n\nEvery adapter that is up. `Link` is how the app classes it; blank means the app does not treat it as a Link.\n\n";
    let rows: Vec<Vec<String>> = all
        .iter()
        .filter(|a| a.up)
        .map(|a| {
            let v4 = interface_row(a.luid, false);
            let metric = v4.map_or("–".into(), |r| {
                format!(
                    "{}{}",
                    r.Metric,
                    if r.UseAutomaticMetric { " auto" } else { "" }
                )
            });
            let weak = v4.map_or("–", |r| if r.WeakHostSend { "yes" } else { "no" });
            let link = links.iter().find(|l| l.id.luid == a.luid);
            vec![
                a.alias.clone(),
                a.description.clone(),
                a.if_type.to_string(),
                a.gateways
                    .iter()
                    .map(|g| redact_ip(*g, raw))
                    .collect::<Vec<_>>()
                    .join(", "),
                a.dns
                    .iter()
                    .map(|g| redact_ip(*g, raw))
                    .collect::<Vec<_>>()
                    .join(", "),
                metric,
                weak.into(),
                link.map_or(String::new(), |l| {
                    format!(
                        "{}{}",
                        kind_name(l.kind),
                        if l.default_route {
                            ", default route"
                        } else {
                            ""
                        }
                    )
                }),
            ]
        })
        .collect();
    out += &table(
        &[
            "Alias",
            "Description",
            "IfType",
            "Gateway",
            "DNS",
            "IPv4 metric",
            "Weak host send",
            "Link",
        ],
        &rows,
    );

    let followed = links
        .iter()
        .find(|l| l.default_route)
        .or_else(|| links.first());
    let carrier = followed
        .filter(|f| f.kind == LinkKind::Vpn)
        .and_then(|_| links.iter().find(|l| l.kind != LinkKind::Vpn));
    // As the app reads system-dns: the first IPv4 DNS server of the default route's adapter.
    let resolver = route(None, CF4.into()).ok().and_then(|(luid, _)| {
        all.iter()
            .find(|a| a.luid == luid)?
            .dns
            .iter()
            .find(|d| d.is_ipv4())
            .copied()
    });
    out += "\n## The app's view\n\n";
    for l in &links {
        out += &format!(
            "- {} ({}): Gateway {}, MAC {}, IPv4 index {}, IPv6 index {}\n",
            l.alias,
            kind_name(l.kind),
            l.id.gateway.map_or("none".into(), |g| redact_ip(g, raw)),
            l.id.gateway_mac
                .map_or("unread".into(), |m| redact_mac(m, raw)),
            l.index_v4,
            l.index_v6,
        );
    }
    out += &format!(
        "\nFollowed Link (default route, nothing pinned): {}. Verdict Gateway Probes from: {}. system-dns: {}.\n",
        followed.map_or("none".into(), |l| l.alias.clone()),
        carrier
            .or(followed)
            .map_or("none".into(), |l| l.alias.clone()),
        resolver.map_or("none".into(), |r| redact_ip(r, raw)),
    );

    let targets = targets(&links, resolver, tailscale_adapter, raw);

    out += "\n## Routes\n\n`GetBestRoute2` per Target: unconstrained, then constrained to each Link. A cell is the interface the route leaves through and its next hop, or the error.\n\n";
    let mut head = vec!["Target".to_string(), "unconstrained".into()];
    head.extend(links.iter().map(|l| l.alias.clone()));
    let rows: Vec<Vec<String>> = targets
        .iter()
        .map(|t| {
            let mut row = vec![t.name.clone()];
            let cell = |luid: Option<u64>| match route(luid, t.ip) {
                Ok((via, hop)) => format!(
                    "{}{}",
                    alias(via),
                    hop.map_or(" on-link".into(), |h| format!(" via {}", redact_ip(h, raw)))
                ),
                Err(e) => format!("err {e} {}", crate::error_name(e)),
            };
            row.push(cell(None));
            row.extend(links.iter().map(|l| cell(Some(l.id.luid))));
            row
        })
        .collect();
    let head_ref: Vec<&str> = head.iter().map(String::as_str).collect();
    out += &table(&head_ref, &rows);

    eprintln!("linkcheck: sending Probes…");
    out += &format!(
        "\n## Sends\n\nEach cell is {TRIES} tries with a {} s timeout: replies of tries, median RTT, then any error. Pinned columns bind the socket to the Link (`IP_UNICAST_IF`) and send ICMP from its address, as the app does; they ignore whether the Link routes the Target.\n\n",
        TIMEOUT.as_secs()
    );
    let cols: Vec<Option<&Link>> = std::iter::once(None)
        .chain(links.iter().map(Some))
        .collect();
    let cells: BTreeMap<(usize, Kind, usize), String> = thread::scope(|s| {
        let mut handles = Vec::new();
        for (ti, t) in targets.iter().enumerate() {
            for &k in &t.kinds {
                for (ci, via) in cols.iter().enumerate() {
                    let via = *via;
                    let ip = t.ip;
                    handles.push((
                        (ti, k, ci),
                        s.spawn(move || {
                            let tries: Vec<Outcome> = (0..TRIES)
                                .map(|_| {
                                    let o = k.send(via, ip);
                                    thread::sleep(Duration::from_millis(200));
                                    o
                                })
                                .collect();
                            summarize(&tries)
                        }),
                    ));
                }
            }
        }
        handles
            .into_iter()
            .map(|(key, h)| (key, h.join().unwrap_or_else(|_| "panic".into())))
            .collect()
    });
    let mut head = vec!["Target".to_string(), "Probe".into(), "unpinned".into()];
    head.extend(links.iter().map(|l| l.alias.clone()));
    let mut rows = Vec::new();
    for (ti, t) in targets.iter().enumerate() {
        for &k in &t.kinds {
            let mut row = vec![t.name.clone(), k.name().into()];
            row.extend((0..cols.len()).map(|ci| cells[&(ti, k, ci)].clone()));
            rows.push(row);
        }
    }
    let head_ref: Vec<&str> = head.iter().map(String::as_str).collect();
    out += &table(&head_ref, &rows);
    out
}

fn windows_version() -> String {
    Command::new("cmd")
        .args(["/c", "ver"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Windows (version unread)".into())
}

/// The Tailscale CLI, from PATH or its default install folder.
fn tailscale_cli(args: &[&str]) -> Option<serde_json::Value> {
    for exe in ["tailscale", r"C:\Program Files\Tailscale\tailscale.exe"] {
        if let Ok(o) = Command::new(exe).args(args).output()
            && o.status.success()
        {
            return serde_json::from_slice(&o.stdout).ok();
        }
    }
    None
}

/// Tailscale's state: backend, exit node, LAN access and DNS preferences.
/// https://tailscale.com/docs/features/exit-nodes
fn tailscale(raw: bool) -> String {
    let mut out = "## Tailscale\n\n".to_string();
    let Some(status) = tailscale_cli(&["status", "--json"]) else {
        return out + "CLI not found or not running.\n\n";
    };
    let prefs = tailscale_cli(&["debug", "prefs"]).unwrap_or_default();
    let s = |v: &serde_json::Value, k: &str| v.get(k).map_or("–".into(), |x| x.to_string());
    let exit = prefs
        .get("ExitNodeID")
        .and_then(|v| v.as_str())
        .is_some_and(|v| !v.is_empty())
        || prefs
            .get("ExitNodeIP")
            .and_then(|v| v.as_str())
            .is_some_and(|v| !v.is_empty());
    out += &format!(
        "- Backend: {}\n- Exit node in use: {}\n- Allow LAN access: {}\n- Use Tailscale DNS (CorpDNS): {}\n- Accept routes (RouteAll): {}\n",
        s(&status, "BackendState"),
        if exit { "yes" } else { "no" },
        s(&prefs, "ExitNodeAllowLANAccess"),
        s(&prefs, "CorpDNS"),
        s(&prefs, "RouteAll"),
    );
    if let Some(ips) = status
        .get("Self")
        .and_then(|v| v.get("TailscaleIPs"))
        .and_then(|v| v.as_array())
    {
        let ips: Vec<String> = ips
            .iter()
            .filter_map(|v| v.as_str()?.parse::<IpAddr>().ok())
            .map(|ip| redact_ip(ip, raw))
            .collect();
        out += &format!("- This device: {}\n", ips.join(", "));
    }
    out + "\n"
}

/// The DNS policy Windows applies, which is where Tailscale's MagicDNS catch-all lives.
/// https://learn.microsoft.com/powershell/module/dnsclient/get-dnsclientnrptpolicy
fn nrpt(raw: bool) -> String {
    let mut out = "## Name Resolution Policy\n\n".to_string();
    let o = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-DnsClientNrptPolicy -Effective | Select-Object Namespace,NameServers | ConvertTo-Json -Compress",
        ])
        .output();
    let Ok(o) = o else {
        return out + "PowerShell did not run.\n\n";
    };
    let text = String::from_utf8_lossy(&o.stdout);
    if text.trim().is_empty() {
        return out + "No rules.\n\n";
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return out + "Unreadable output.\n\n";
    };
    let rules = match v {
        serde_json::Value::Array(a) => a,
        one => vec![one],
    };
    let list = |v: &serde_json::Value| -> Vec<String> {
        match v {
            serde_json::Value::Array(a) => a
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect(),
            serde_json::Value::String(s) => vec![s.clone()],
            _ => vec![],
        }
    };
    for r in &rules {
        let ns: Vec<String> = list(&r["Namespace"])
            .into_iter()
            // A corporate suffix names the employer; only the catch-all matters here.
            .map(|n| {
                if raw || n == "." {
                    n
                } else {
                    "(a domain)".into()
                }
            })
            .collect();
        let servers: Vec<String> = list(&r["NameServers"])
            .into_iter()
            .map(|s| s.parse().map_or(s.clone(), |ip| redact_ip(ip, raw)))
            .collect();
        out += &format!("- {} → {}\n", ns.join(", "), servers.join(", "));
    }
    out + "\n"
}

/// Waits for `n` wakes and reports each: how far each clock moved, then how long until each
/// Link was listed again and answered from its Gateway and 1.1.1.1.
pub fn resume(args: &Args, n: u32) {
    println!(
        "# linkcheck --resume{}\n\n{}. linkcheck {}.\n",
        args.label
            .as_ref()
            .map_or(String::new(), |l| format!(": {l}")),
        windows_version(),
        env!("CARGO_PKG_VERSION"),
    );
    eprintln!("linkcheck: waiting for the machine to sleep {n} times; Ctrl+C to stop.");
    let tick = Duration::from_millis(250);
    let mut last_wall = SystemTime::now();
    let mut last = clocks();
    let mut seen = 0;
    while seen < n {
        thread::sleep(tick);
        let wall = SystemTime::now();
        let now = clocks();
        let wall_gap = wall
            .duration_since(last_wall)
            .unwrap_or_default()
            .as_secs_f64();
        let biased = (now.0 - last.0) as f64 / 1e7;
        let unbiased = (now.1 - last.1) as f64 / 1e7;
        let slept = biased - unbiased;
        (last_wall, last) = (wall, now);
        if wall_gap < 2.0 && slept < 1.0 {
            continue;
        }
        seen += 1;
        let at = wall
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        println!(
            "## Wake {seen} at Unix {at}\n\nSince the last 0.25 s tick: wall {wall_gap:.1} s, interrupt time {biased:.1} s, unbiased {unbiased:.1} s. The app counts {slept:.1} s as slept{}.\n",
            if slept > 1.0 {
                ""
            } else {
                ": too little to detect, so the unbiased clock kept running"
            }
        );
        println!("{}", settle(args.raw));
    }
}

/// Polls every 0.2 s for up to 60 s after a wake, noting when each first happened.
fn settle(raw: bool) -> String {
    let t0 = Instant::now();
    let limit = Duration::from_secs(60);
    let mut first: BTreeMap<String, f64> = BTreeMap::new();
    let note = |what: String, first: &mut BTreeMap<String, f64>| {
        first
            .entry(what)
            .or_insert_with(|| t0.elapsed().as_secs_f64());
    };
    while t0.elapsed() < limit {
        let links = probe::links();
        if !links.is_empty() {
            note("Links listed".into(), &mut first);
        }
        if probe::family_up(false) {
            note("IPv4 default route".into(), &mut first);
        }
        let mut sends: Vec<(String, Option<Link>, IpAddr, Kind)> =
            vec![("1.1.1.1 ICMP unpinned".into(), None, CF4.into(), Kind::Icmp)];
        for l in &links {
            note(format!("{} listed", l.alias), &mut first);
            if let Some(gw) = l.id.gateway {
                sends.push((
                    format!("{} Gateway ({}) ICMP", l.alias, redact_ip(gw, raw)),
                    Some(l.clone()),
                    gw,
                    Kind::Icmp,
                ));
            }
            sends.push((
                format!("{} 1.1.1.1 ICMP", l.alias),
                Some(l.clone()),
                CF4.into(),
                Kind::Icmp,
            ));
            sends.push((
                format!("{} 1.1.1.1 DNS", l.alias),
                Some(l.clone()),
                CF4.into(),
                Kind::Dns,
            ));
        }
        sends.retain(|(name, ..)| !first.contains_key(name));
        let done: Vec<String> = thread::scope(|s| {
            let hs: Vec<_> = sends
                .iter()
                .map(|(name, via, ip, k)| {
                    s.spawn(move || {
                        matches!(k.send(via.as_ref(), *ip), Outcome::Reply(_)).then(|| name.clone())
                    })
                })
                .collect();
            hs.into_iter()
                .filter_map(|h| h.join().ok().flatten())
                .collect()
        });
        for d in done {
            note(d, &mut first);
        }
        let waiting = links.is_empty()
            || links.iter().any(|l| {
                !first.contains_key(&format!("{} 1.1.1.1 ICMP", l.alias))
                    || l.id.gateway.is_some()
                        && !first
                            .keys()
                            .any(|k| k.starts_with(&format!("{} Gateway", l.alias)))
            });
        if !waiting && t0.elapsed() > Duration::from_secs(5) {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    let mut rows: Vec<(f64, String)> = first.into_iter().map(|(k, v)| (v, k)).collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let rows: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(t, what)| vec![format!("{t:.1}"), what])
        .collect();
    format!(
        "Seconds after the wake was noticed until each first happened; what is missing never did within {} s.\n\n{}",
        limit.as_secs(),
        table(&["s", "First"], &rows)
    )
}

fn sockaddr_in6(a: Ipv6Addr) -> SOCKADDR_IN6 {
    SOCKADDR_IN6 {
        sin6_family: AF_INET6,
        sin6_addr: IN6_ADDR {
            u: IN6_ADDR_0 { Byte: a.octets() },
        },
        ..Default::default()
    }
}

fn sockaddr_inet(to: IpAddr) -> SOCKADDR_INET {
    let mut dest = SOCKADDR_INET::default();
    match to {
        IpAddr::V4(a) => {
            dest.Ipv4 = SOCKADDR_IN {
                sin_family: AF_INET,
                sin_addr: IN_ADDR {
                    S_un: IN_ADDR_0 {
                        S_addr: u32::from_ne_bytes(a.octets()),
                    },
                },
                ..Default::default()
            }
        }
        IpAddr::V6(a) => dest.Ipv6 = sockaddr_in6(a),
    }
    dest
}

fn inet_ip(a: &SOCKADDR_INET) -> Option<IpAddr> {
    unsafe {
        match a.si_family {
            AF_INET => Some(Ipv4Addr::from(a.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()).into()),
            AF_INET6 => Some(Ipv6Addr::from(a.Ipv6.sin6_addr.u.Byte).into()),
            _ => None,
        }
    }
}

fn socket_ip(sa: *const SOCKADDR) -> Option<IpAddr> {
    let family = unsafe { sa.as_ref() }?.sa_family;
    if family == AF_INET {
        let sin = unsafe { &*(sa as *const SOCKADDR_IN) };
        Some(Ipv4Addr::from(unsafe { sin.sin_addr.S_un.S_addr }.to_ne_bytes()).into())
    } else if family == AF_INET6 {
        let sin6 = unsafe { &*(sa as *const SOCKADDR_IN6) };
        Some(Ipv6Addr::from(unsafe { sin6.sin6_addr.u.Byte }).into())
    } else {
        None
    }
}

fn wide(p: windows_sys::core::PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let len = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}
