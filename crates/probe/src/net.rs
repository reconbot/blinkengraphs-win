use crate::dns;
use model::Link;
use std::{
    hash::{BuildHasher, RandomState},
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

/// A refused connection still proves the host answered, so it counts as a reply.
#[cfg(windows)]
pub fn tcp(via: Option<&Link>, addr: SocketAddr, timeout: Duration) -> Option<Duration> {
    match crate::win::tcp_connect(via, addr, timeout, None) {
        crate::win::Connect::Reply(d) => Some(d),
        _ => None,
    }
}

/// Off Windows a socket follows the routing table; binding to a device needs root.
#[cfg(not(windows))]
pub fn tcp(via: Option<&Link>, addr: SocketAddr, timeout: Duration) -> Option<Duration> {
    let _ = via;
    let start = Instant::now();
    match std::net::TcpStream::connect_timeout(&addr, timeout) {
        Ok(_) => Some(start.elapsed()),
        Err(e) if e.kind() == ErrorKind::ConnectionRefused => Some(start.elapsed()),
        Err(_) => None,
    }
}

/// On a connected UDP socket an ICMP port-unreachable surfaces as a reset (Windows) or refused (Unix)
/// on the next receive, which proves the host is up. Silence is loss: an open port that never replies
/// is indistinguishable from a dropped packet.
pub fn udp(via: Option<&Link>, addr: SocketAddr, timeout: Duration) -> Option<Duration> {
    let socket = bound(via, addr)?;
    socket.connect(addr).ok()?;
    socket.set_read_timeout(Some(timeout)).ok()?;
    let start = Instant::now();
    socket.send(b"blinkengraphs").ok()?;
    let mut buf = [0u8; 512];
    match socket.recv(&mut buf) {
        Ok(_) => Some(start.elapsed()),
        Err(e)
            if matches!(
                e.kind(),
                ErrorKind::ConnectionReset | ErrorKind::ConnectionRefused
            ) =>
        {
            Some(start.elapsed())
        }
        Err(_) => None,
    }
}

fn bound(via: Option<&Link>, addr: SocketAddr) -> Option<UdpSocket> {
    let local: IpAddr = match addr {
        SocketAddr::V4(_) => Ipv4Addr::UNSPECIFIED.into(),
        SocketAddr::V6(_) => Ipv6Addr::UNSPECIFIED.into(),
    };
    let socket = UdpSocket::bind((local, 0)).ok()?;
    #[cfg(windows)]
    if let Some(link) = via {
        use std::os::windows::io::AsRawSocket;
        crate::win::unicast_if(socket.as_raw_socket() as _, addr.is_ipv6(), link).then_some(())?;
    }
    #[cfg(not(windows))]
    let _ = via;
    Some(socket)
}

pub fn dns(
    via: Option<&Link>,
    addr: SocketAddr,
    timeout: Duration,
    name: &str,
) -> Option<Duration> {
    let socket = bound(via, addr)?;
    socket.connect(addr).ok()?;
    let id = random_u16();
    let query = dns::encode_query(id, name);
    let start = Instant::now();
    let deadline = start + timeout;
    socket.send(&query).ok()?;
    let mut buf = [0u8; 512];
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        if left.is_zero() {
            return None;
        }
        socket.set_read_timeout(Some(left)).ok()?;
        match socket.recv(&mut buf) {
            Ok(n) => {
                if dns::parse_response(&buf[..n]).is_some_and(|h| h.id == id) {
                    return Some(start.elapsed());
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

/// 12 characters from `a`–`z` and `0`–`9`, fresh each call: a `system-dns` label.
pub fn random_label() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut bits = RandomState::new().hash_one(Instant::now());
    (0..12)
        .map(|i| {
            if i == 6 {
                bits = RandomState::new().hash_one((Instant::now(), bits));
            }
            let c = CHARS[(bits % CHARS.len() as u64) as usize] as char;
            bits /= CHARS.len() as u64;
            c
        })
        .collect()
}

fn random_u16() -> u16 {
    RandomState::new().hash_one(Instant::now()) as u16
}

/// RTTs are recorded in whole milliseconds, `⌊d + 0.5⌋` (spec `rules/probe-stats.md`).
pub fn ms(d: Duration) -> f32 {
    (d.as_secs_f64() * 1000.0 + 0.5).floor() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    const T: Duration = Duration::from_secs(1);

    #[test]
    fn udp_echo_counts_as_reply() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let echo = std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            server.send_to(&buf[..n], from).unwrap();
        });
        assert!(udp(None, addr, T).is_some());
        echo.join().unwrap();
    }

    #[test]
    fn udp_closed_port_counts_as_reply() {
        let closed = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(
            udp(None, closed, T).is_some(),
            "port unreachable should prove the host answered"
        );
    }

    #[test]
    fn tcp_reply_from_listener() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        assert!(tcp(None, l.local_addr().unwrap(), T).is_some());
    }

    #[test]
    fn tcp_refused_counts_as_reply() {
        let addr = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(tcp(None, addr, T).is_some());
    }

    #[test]
    fn dns_reply_with_matching_id() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let echo = std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            let mut wrong = buf[..n].to_vec();
            wrong[0] ^= 0xff;
            wrong[2] |= 0x80;
            server.send_to(&wrong, from).unwrap();
            buf[2] |= 0x80;
            server.send_to(&buf[..n], from).unwrap();
        });
        assert!(dns(None, addr, T, "cloudflare.com").is_some());
        echo.join().unwrap();
    }

    #[test]
    fn dns_silence_is_loss() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        assert!(
            dns(
                None,
                server.local_addr().unwrap(),
                Duration::from_millis(100),
                "cloudflare.com"
            )
            .is_none()
        );
    }
}
