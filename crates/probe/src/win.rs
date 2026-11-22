use crate::trace::Hop;
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io::ErrorKind,
    mem::{size_of, zeroed},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4},
    os::windows::io::AsRawSocket,
    ptr::{null, null_mut},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_BUFFER_OVERFLOW, ERROR_IO_PENDING, GetLastError, HANDLE,
        INVALID_HANDLE_VALUE, NO_ERROR, WAIT_OBJECT_0,
    },
    NetworkManagement::IpHelper::{
        GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_FRIENDLY_NAME, GAA_FLAG_SKIP_MULTICAST,
        GAA_FLAG_SKIP_UNICAST, GetAdaptersAddresses, GetBestRoute2, ICMP_ECHO_REPLY,
        ICMPV6_ECHO_REPLY_LH, IP_ADAPTER_ADDRESSES_LH, IP_OPTION_INFORMATION, IP_REQ_TIMED_OUT,
        Icmp6CreateFile, Icmp6ParseReplies, Icmp6SendEcho2, IcmpCloseHandle, IcmpCreateFile,
        IcmpParseReplies, IcmpSendEcho2, MIB_IPFORWARD_ROW2,
    },
    Networking::WinSock::{
        AF_INET, AF_INET6, AF_UNSPEC, GetNameInfoW, ICMP_ERROR_INFO, IN_ADDR, IN_ADDR_0, IN6_ADDR,
        IN6_ADDR_0, IPPROTO_TCP, NI_MAXHOST, NI_NAMEREQD, SIO_TCP_INITIAL_RTO, SOCKADDR,
        SOCKADDR_IN, SOCKADDR_IN6, SOCKADDR_INET, SOCKET, TCP_FAIL_CONNECT_ON_ICMP_ERROR,
        TCP_ICMP_ERROR_INFO, TCP_INITIAL_RTO_NO_SYN_RETRANSMISSIONS, TCP_INITIAL_RTO_PARAMETERS,
        WSAIoctl, getsockopt, setsockopt,
    },
    System::Threading::{CreateEventW, WaitForMultipleObjects},
};

pub use windows_sys::Win32::NetworkManagement::IpHelper::IP_SUCCESS;

const PAYLOAD: [u8; 32] = [0; 32];
// ICMP_ECHO_REPLY + payload + 8 bytes of ICMP error data + IO_STATUS_BLOCK, per the IcmpSendEcho2 docs; plus slack for quoted headers.
const REPLY_WORDS: usize = (size_of::<ICMP_ECHO_REPLY>() + PAYLOAD.len() + 8 + 16 + 128) / 8;
const MAXIMUM_WAIT_OBJECTS: usize = 64;
const GRACE: Duration = Duration::from_millis(250);

pub struct Echo {
    pub from: IpAddr,
    pub status: u32,
    pub rtt: Duration,
}

/// ICMP handles for IPv4 and, when the IPv6 stack is present, IPv6.
pub struct Icmp {
    v4: HANDLE,
    v6: Option<HANDLE>,
}

// The ICMP handles are kernel object handles, usable from any thread.
unsafe impl Send for Icmp {}

impl Drop for Icmp {
    fn drop(&mut self) {
        unsafe { IcmpCloseHandle(self.v4) };
        if let Some(h) = self.v6 {
            unsafe { IcmpCloseHandle(h) };
        }
    }
}

fn valid(h: HANDLE) -> Option<HANDLE> {
    (h != INVALID_HANDLE_VALUE && !h.is_null()).then_some(h)
}

fn sockaddr_in6(ip: Ipv6Addr) -> SOCKADDR_IN6 {
    SOCKADDR_IN6 {
        sin6_family: AF_INET6,
        sin6_addr: IN6_ADDR {
            u: IN6_ADDR_0 { Byte: ip.octets() },
        },
        ..Default::default()
    }
}

struct Request {
    v6: bool,
    event: HANDLE,
    options: Box<IP_OPTION_INFORMATION>,
    // Icmp6SendEcho2 may read these after returning ERROR_IO_PENDING, like `options`.
    addrs: Box<[SOCKADDR_IN6; 2]>,
    reply: Box<[u64; REPLY_WORDS]>,
    sent: Instant,
    done: Option<Instant>,
    pending: bool,
}

impl Icmp {
    pub fn new() -> Option<Icmp> {
        Some(Icmp {
            v4: valid(unsafe { IcmpCreateFile() })?,
            v6: valid(unsafe { Icmp6CreateFile() }),
        })
    }

    /// Sends every `(destination, ttl)` echo at once and waits up to `timeout` for all of them.
    /// For IPv6 the TTL is the hop limit.
    /// https://learn.microsoft.com/windows/win32/api/icmpapi/nf-icmpapi-icmpsendecho2
    /// https://learn.microsoft.com/windows/win32/api/icmpapi/nf-icmpapi-icmp6sendecho2
    pub fn batch(&self, requests: &[(IpAddr, u8)], timeout: Duration) -> Vec<Option<Echo>> {
        let timeout_ms = timeout.as_millis().max(1) as u32;
        let mut reqs: Vec<Request> = requests
            .iter()
            .map(|&(dst, ttl)| {
                let mut r = Request {
                    v6: dst.is_ipv6(),
                    addrs: Box::new([SOCKADDR_IN6::default(); 2]),
                    event: unsafe { CreateEventW(null(), 1, 0, null()) },
                    options: Box::new(IP_OPTION_INFORMATION {
                        Ttl: ttl,
                        ..Default::default()
                    }),
                    reply: Box::new([0; REPLY_WORDS]),
                    sent: Instant::now(),
                    done: None,
                    pending: false,
                };
                if r.event.is_null() {
                    return r;
                }
                let rc = match dst {
                    IpAddr::V4(dst) => unsafe {
                        IcmpSendEcho2(
                            self.v4,
                            r.event,
                            None,
                            null(),
                            u32::from_ne_bytes(dst.octets()),
                            PAYLOAD.as_ptr().cast(),
                            PAYLOAD.len() as u16,
                            &*r.options,
                            r.reply.as_mut_ptr().cast(),
                            size_of::<[u64; REPLY_WORDS]>() as u32,
                            timeout_ms,
                        )
                    },
                    IpAddr::V6(dst) => {
                        let Some(h) = self.v6 else { return r };
                        // Source :: lets the stack pick the outgoing address.
                        r.addrs[0] = sockaddr_in6(Ipv6Addr::UNSPECIFIED);
                        r.addrs[1] = sockaddr_in6(dst);
                        unsafe {
                            Icmp6SendEcho2(
                                h,
                                r.event,
                                None,
                                null(),
                                &r.addrs[0],
                                &r.addrs[1],
                                PAYLOAD.as_ptr().cast(),
                                PAYLOAD.len() as u16,
                                &*r.options,
                                r.reply.as_mut_ptr().cast(),
                                size_of::<[u64; REPLY_WORDS]>() as u32,
                                timeout_ms,
                            )
                        }
                    }
                };
                if rc != 0 {
                    r.done = Some(Instant::now());
                } else {
                    r.pending = unsafe { GetLastError() } == ERROR_IO_PENDING;
                }
                r
            })
            .collect();

        // Wait for any, one at a time, so each reply gets its own sub-ms timestamp.
        let deadline = Instant::now() + timeout + GRACE;
        loop {
            let waiting: Vec<usize> = (0..reqs.len())
                .filter(|&i| reqs[i].pending)
                .take(MAXIMUM_WAIT_OBJECTS)
                .collect();
            if waiting.is_empty() {
                break;
            }
            let handles: Vec<HANDLE> = waiting.iter().map(|&i| reqs[i].event).collect();
            let left = deadline.saturating_duration_since(Instant::now());
            let rc = unsafe {
                WaitForMultipleObjects(
                    handles.len() as u32,
                    handles.as_ptr(),
                    0,
                    left.as_millis() as u32,
                )
            };
            let k = rc.wrapping_sub(WAIT_OBJECT_0) as usize;
            if k >= handles.len() {
                break;
            }
            let r = &mut reqs[waiting[k]];
            r.pending = false;
            r.done = Some(Instant::now());
        }

        reqs.into_iter()
            .map(|r| {
                unsafe { CloseHandle(r.event) };
                if r.pending {
                    // The driver may still write into these buffers; leaking is the only safe option.
                    Box::leak(r.reply);
                    Box::leak(r.options);
                    Box::leak(r.addrs);
                    return None;
                }
                parse(r)
            })
            .collect()
    }
}

fn parse(mut r: Request) -> Option<Echo> {
    let done = r.done?;
    let size = size_of::<[u64; REPLY_WORDS]>() as u32;
    let (n, from, status, coarse_ms) = if r.v6 {
        let n = unsafe { Icmp6ParseReplies(r.reply.as_mut_ptr().cast(), size) };
        // The address struct is packed; read the reply unaligned.
        let reply: ICMPV6_ECHO_REPLY_LH = unsafe {
            r.reply
                .as_ptr()
                .cast::<ICMPV6_ECHO_REPLY_LH>()
                .read_unaligned()
        };
        let words = reply.Address.sin6_addr;
        let mut bytes = [0u8; 16];
        for (i, w) in words.iter().enumerate() {
            bytes[2 * i..2 * i + 2].copy_from_slice(&w.to_ne_bytes());
        }
        let from = Ipv6Addr::from(bytes);
        let addr_zero = from.is_unspecified();
        (
            n,
            (!addr_zero).then_some(IpAddr::V6(from)),
            reply.Status,
            reply.RoundTripTime,
        )
    } else {
        let n = unsafe { IcmpParseReplies(r.reply.as_mut_ptr().cast(), size) };
        let reply: ICMP_ECHO_REPLY = unsafe { r.reply.as_ptr().cast::<ICMP_ECHO_REPLY>().read() };
        let from =
            (reply.Address != 0).then(|| IpAddr::V4(Ipv4Addr::from(reply.Address.to_ne_bytes())));
        (n, from, reply.Status, reply.RoundTripTime)
    };
    // The parse functions return 0 for error replies such as TTL expired, yet still fill in Status and Address.
    if n == 0 && (status == 0 || status == IP_REQ_TIMED_OUT || from.is_none()) {
        return None;
    }
    let measured = done.duration_since(r.sent);
    let coarse = Duration::from_millis(coarse_ms.into());
    // A late wake-up (another reply was being handled) inflates `measured`; RoundTripTime is then closer.
    let rtt = if measured <= coarse + Duration::from_millis(1) {
        measured
    } else {
        coarse
    };
    Some(Echo {
        from: from?,
        status,
        rtt,
    })
}

pub enum Connect {
    Reply(Duration),
    Icmp {
        from: Option<IpAddr>,
        after: Duration,
    },
    Lost,
}

pub fn tcp_connect(addr: SocketAddr, timeout: Duration, ttl: Option<u8>) -> Connect {
    let Ok(socket) = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))
    else {
        return Connect::Lost;
    };
    let raw = socket.as_raw_socket() as SOCKET;
    // Windows retransmits the SYN after an RST for ~2 s, which would turn a closed port into a loss.
    // https://learn.microsoft.com/windows/win32/winsock/sio-tcp-initial-rto
    let rto = TCP_INITIAL_RTO_PARAMETERS {
        Rtt: u16::MAX,
        MaxSynRetransmissions: TCP_INITIAL_RTO_NO_SYN_RETRANSMISSIONS as u8,
    };
    let mut returned = 0u32;
    unsafe {
        WSAIoctl(
            raw,
            SIO_TCP_INITIAL_RTO,
            (&rto as *const TCP_INITIAL_RTO_PARAMETERS).cast(),
            size_of::<TCP_INITIAL_RTO_PARAMETERS>() as u32,
            null_mut(),
            0,
            &mut returned,
            null_mut(),
            None,
        )
    };
    if let Some(ttl) = ttl {
        let on: u32 = 1;
        let rc = unsafe {
            setsockopt(
                raw,
                IPPROTO_TCP,
                TCP_FAIL_CONNECT_ON_ICMP_ERROR,
                (&on as *const u32).cast(),
                4,
            )
        };
        if socket.set_ttl_v4(ttl.into()).is_err() || rc != 0 {
            return Connect::Lost;
        }
    }
    let start = Instant::now();
    match socket.connect_timeout(&addr.into(), timeout) {
        Ok(()) => Connect::Reply(start.elapsed()),
        Err(e) if e.kind() == ErrorKind::ConnectionRefused => Connect::Reply(start.elapsed()),
        Err(e) if ttl.is_some() && e.kind() != ErrorKind::TimedOut => Connect::Icmp {
            after: start.elapsed(),
            from: icmp_error_source(raw),
        },
        Err(_) => Connect::Lost,
    }
}

/// https://learn.microsoft.com/windows/win32/winsock/ipproto-tcp-socket-options (TCP_ICMP_ERROR_INFO, Windows 10 2004+)
fn icmp_error_source(raw: SOCKET) -> Option<IpAddr> {
    let mut info = ICMP_ERROR_INFO::default();
    let mut len = size_of::<ICMP_ERROR_INFO>() as i32;
    let rc = unsafe {
        getsockopt(
            raw,
            IPPROTO_TCP,
            TCP_ICMP_ERROR_INFO,
            (&mut info as *mut ICMP_ERROR_INFO).cast(),
            &mut len,
        )
    };
    if rc != 0 || len == 0 {
        return None;
    }
    sockaddr_inet(&info.srcaddress).filter(|a| !a.is_unspecified())
}

/// Unverified technique (ADR 0006); a hop whose ICMP error carries no source reports `addr: None`.
pub fn tcp_trace(ip: Ipv4Addr, port: u16, max_ttl: u8, timeout: Duration) -> Vec<Hop> {
    let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));
    thread::scope(|s| {
        let handles: Vec<_> = (1..=max_ttl)
            .map(|ttl| (ttl, s.spawn(move || tcp_connect(addr, timeout, Some(ttl)))))
            .collect();
        handles
            .into_iter()
            .map(|(ttl, h)| {
                let (addr, rtt, reached) = match h.join().unwrap_or(Connect::Lost) {
                    Connect::Reply(d) => (Some(IpAddr::V4(ip)), Some(d), true),
                    Connect::Icmp { from, after } => (from, Some(after), false),
                    Connect::Lost => (None, None, false),
                };
                Hop {
                    ttl,
                    addr,
                    rtt_ms: rtt.map(crate::net::ms),
                    reached,
                }
            })
            .collect()
    })
}

fn sockaddr_inet(a: &SOCKADDR_INET) -> Option<IpAddr> {
    unsafe {
        match a.si_family {
            AF_INET => Some(Ipv4Addr::from(a.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()).into()),
            AF_INET6 => Some(Ipv6Addr::from(a.Ipv6.sin6_addr.u.Byte).into()),
            _ => None,
        }
    }
}

/// The route the stack would use to reach `to`, if any.
pub(crate) fn route_to(to: IpAddr) -> Option<MIB_IPFORWARD_ROW2> {
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
    let mut row: MIB_IPFORWARD_ROW2 = unsafe { zeroed() };
    let mut source = SOCKADDR_INET::default();
    let rc = unsafe { GetBestRoute2(null(), 0, null(), &dest, 0, &mut row, &mut source) };
    (rc == NO_ERROR).then_some(row)
}

pub(crate) fn default_route() -> Option<MIB_IPFORWARD_ROW2> {
    route_to(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)))
}

/// Whether the stack has a route for this IP version, i.e. the path can reach its Targets.
pub(crate) fn family_up(v6: bool) -> bool {
    let probe: IpAddr = if v6 {
        "2606:4700:4700::1111".parse().unwrap()
    } else {
        Ipv4Addr::new(1, 1, 1, 1).into()
    };
    route_to(probe).is_some()
}

/// https://learn.microsoft.com/windows/win32/api/netioapi/nf-netioapi-getbestroute2
pub fn gateway() -> Option<IpAddr> {
    sockaddr_inet(&default_route()?.NextHop).filter(|a| !a.is_unspecified())
}

/// First IPv4 DNS server of the adapter that carries the default route.
/// https://learn.microsoft.com/windows/win32/api/iphlpapi/nf-iphlpapi-getadaptersaddresses
pub fn system_resolver() -> Option<IpAddr> {
    let if_index = default_route()?.InterfaceIndex;
    let flags = GAA_FLAG_SKIP_UNICAST
        | GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_FRIENDLY_NAME;
    let mut size: u32 = 16 * 1024;
    let mut buf: Vec<u64>;
    // The adapter list can grow between the sizing call and the real one.
    let mut tries = 0;
    loop {
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
            ERROR_BUFFER_OVERFLOW if tries < 3 => tries += 1,
            _ => return None,
        }
    }
    let mut adapter = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while let Some(a) = unsafe { adapter.as_ref() } {
        let index = unsafe { a.Anonymous1.Anonymous.IfIndex };
        if index == if_index || a.Ipv6IfIndex == if_index {
            let mut dns = a.FirstDnsServerAddress;
            while let Some(d) = unsafe { dns.as_ref() } {
                let sa = d.Address.lpSockaddr;
                if !sa.is_null() && unsafe { (*sa).sa_family } == AF_INET {
                    let sin = unsafe { &*(sa as *const SOCKADDR_IN) };
                    let ip = Ipv4Addr::from(unsafe { sin.sin_addr.S_un.S_addr }.to_ne_bytes());
                    return Some(ip.into());
                }
                dns = d.Next;
            }
            return None;
        }
        adapter = a.Next;
    }
    None
}

/// The PTR name for `addr`, or `None` when it has none.
/// https://learn.microsoft.com/windows/win32/api/ws2tcpip/nf-ws2tcpip-getnameinfow
pub fn reverse_dns(addr: IpAddr) -> Option<String> {
    // GetNameInfoW needs Winsock started; std starts it with its first socket.
    static WINSOCK: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    WINSOCK.get_or_init(|| drop(std::net::UdpSocket::bind("127.0.0.1:0")));

    let mut host = [0u16; NI_MAXHOST as usize];
    let rc = match addr {
        IpAddr::V4(a) => {
            let sa = SOCKADDR_IN {
                sin_family: AF_INET,
                sin_addr: IN_ADDR {
                    S_un: IN_ADDR_0 {
                        S_addr: u32::from_ne_bytes(a.octets()),
                    },
                },
                ..Default::default()
            };
            unsafe {
                GetNameInfoW(
                    (&raw const sa).cast::<SOCKADDR>(),
                    size_of::<SOCKADDR_IN>() as i32,
                    host.as_mut_ptr(),
                    host.len() as u32,
                    null_mut(),
                    0,
                    NI_NAMEREQD as i32,
                )
            }
        }
        IpAddr::V6(a) => {
            let sa = sockaddr_in6(a);
            unsafe {
                GetNameInfoW(
                    (&raw const sa).cast::<SOCKADDR>(),
                    size_of::<SOCKADDR_IN6>() as i32,
                    host.as_mut_ptr(),
                    host.len() as u32,
                    null_mut(),
                    0,
                    NI_NAMEREQD as i32,
                )
            }
        }
    };
    if rc != 0 {
        return None;
    }
    let len = host.iter().position(|&c| c == 0).unwrap_or(host.len());
    Some(String::from_utf16_lossy(&host[..len]))
}
