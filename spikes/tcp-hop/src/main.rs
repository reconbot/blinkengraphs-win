//! M0 spike (ADR 0002): can a standard user trace TCP hops via IP_TTL +
//! TCP_FAIL_CONNECT_ON_ICMP_ERROR + TCP_ICMP_ERROR_INFO? Prints a markdown table beside an ICMP trace.
//! Usage: tcp-hop [ipv4=1.1.1.1] [port=443] [max_ttl=20]

#[cfg(not(windows))]
fn main() {
    eprintln!("windows only");
}

#[cfg(windows)]
fn main() {
    use std::net::Ipv4Addr;
    let mut args = std::env::args().skip(1);
    let target: Ipv4Addr = args.next().as_deref().unwrap_or("1.1.1.1").parse().unwrap();
    let port: u16 = args.next().as_deref().unwrap_or("443").parse().unwrap();
    let max_ttl: u8 = args.next().as_deref().unwrap_or("20").parse().unwrap();

    println!("Target `{target}:{port}`\n\n| TTL | ICMP | TCP |\n|---|---|---|");
    for ttl in 1..=max_ttl {
        let icmp = win::icmp_hop(target, ttl);
        let tcp = win::tcp_hop(target, port, ttl);
        println!("| {ttl} | {icmp} | {tcp} |");
        if icmp.starts_with("reached") && tcp.starts_with("reached") {
            break;
        }
    }
}

#[cfg(windows)]
mod win {
    use socket2::{Domain, Protocol, SockAddr, Socket, Type};
    use std::{
        mem::size_of,
        net::{Ipv4Addr, SocketAddrV4},
        os::windows::io::AsRawSocket,
        ptr::null,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{NetworkManagement::IpHelper::*, Networking::WinSock::*};

    const TIMEOUT: Duration = Duration::from_millis(1000);

    pub fn icmp_hop(target: Ipv4Addr, ttl: u8) -> String {
        unsafe {
            let handle = IcmpCreateFile();
            let options = IP_OPTION_INFORMATION {
                Ttl: ttl,
                ..Default::default()
            };
            let payload = [0u8; 32];
            let mut reply = [0u8; size_of::<ICMP_ECHO_REPLY>() + 32 + 8 + 64];
            let n = IcmpSendEcho2(
                handle,
                std::ptr::null_mut(),
                None,
                null(),
                u32::from_ne_bytes(target.octets()),
                payload.as_ptr().cast(),
                payload.len() as u16,
                &options,
                reply.as_mut_ptr().cast(),
                reply.len() as u32,
                TIMEOUT.as_millis() as u32,
            );
            IcmpCloseHandle(handle);
            if n == 0 {
                return "timeout".into();
            }
            let r = &*(reply.as_ptr() as *const ICMP_ECHO_REPLY);
            let from = Ipv4Addr::from(r.Address.to_ne_bytes());
            match r.Status {
                0 => format!("reached {from} {} ms", r.RoundTripTime),
                IP_TTL_EXPIRED_TRANSIT => format!("{from} {} ms", r.RoundTripTime),
                s => format!("{from} status {s}"),
            }
        }
    }

    pub fn tcp_hop(target: Ipv4Addr, port: u16, ttl: u8) -> String {
        let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
        socket.set_ttl_v4(ttl as u32).unwrap();
        let raw = socket.as_raw_socket() as SOCKET;
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
        if rc != 0 {
            return format!("setsockopt failed {}", unsafe { WSAGetLastError() });
        }
        let start = Instant::now();
        let addr = SockAddr::from(SocketAddrV4::new(target, port));
        match socket.connect_timeout(&addr, TIMEOUT) {
            Ok(()) => format!("reached {} ms", start.elapsed().as_millis()),
            Err(e) if e.raw_os_error() == Some(WSAEHOSTUNREACH) => {
                let elapsed = start.elapsed().as_millis();
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
                    return format!("unreachable, no icmp info ({})", unsafe {
                        WSAGetLastError()
                    });
                }
                let from = unsafe {
                    Ipv4Addr::from(info.srcaddress.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes())
                };
                format!(
                    "{from} {elapsed} ms (icmp type {} code {})",
                    info.r#type, info.code
                )
            }
            Err(e) => format!("{}", e.kind()),
        }
    }
}
