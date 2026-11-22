//! Captive Portal detection (spec `rules/verdict.md`, input `captive_portal`): the OS says the
//! network is captive, or a plain-HTTP GET of `captive_portal_url` answers with anything but
//! 204 and an empty body. Plain HTTP because a portal can intercept it but cannot answer
//! HTTPS without a certificate error.

use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

/// Whether a sign-in page holds back the internet. `None` when neither the OS nor the GET
/// could tell, e.g. no network at all; the Verdict then has other Findings to give.
pub fn check(url: &str, timeout: Duration) -> Option<bool> {
    if os_says_captive() {
        return Some(true);
    }
    get(url, timeout).map(|r| is_portal(&r))
}

fn os_says_captive() -> bool {
    #[cfg(windows)]
    return crate::win::captive_hint();
    #[cfg(not(windows))]
    false
}

/// True unless `response` is HTTP status 204 with an empty body.
pub fn is_portal(response: &[u8]) -> bool {
    let Some(split) = response.windows(4).position(|w| w == b"\r\n\r\n") else {
        return true;
    };
    let head = String::from_utf8_lossy(&response[..split]);
    let status = head.lines().next().unwrap_or_default();
    let code = status.split_whitespace().nth(1);
    code != Some("204") || response.len() > split + 4
}

/// The raw response to `GET url` over HTTP/1.1, read until the server closes.
fn get(url: &str, timeout: Duration) -> Option<Vec<u8>> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let path = if path.is_empty() { "/" } else { path };
    let host = authority.split(':').next()?;
    let target = if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    let addr = target.to_socket_addrs().ok()?.next()?;
    let mut s = TcpStream::connect_timeout(&addr, timeout).ok()?;
    s.set_read_timeout(Some(timeout)).ok()?;
    s.set_write_timeout(Some(timeout)).ok()?;
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: blinkengraphs\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    // A portal's page can be large; the status line and whether a body exists are enough.
    // Keep what arrived even if the connection then resets: portals close abruptly.
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while out.len() < 16 * 1024 {
        match s.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn an_empty_204_is_open_internet() {
        assert!(!is_portal(
            b"HTTP/1.1 204 No Content\r\nServer: cloudflare\r\n\r\n"
        ));
    }

    #[test]
    fn anything_else_is_a_portal() {
        assert!(is_portal(b"HTTP/1.1 200 OK\r\n\r\n<html>Sign in</html>"));
        assert!(is_portal(
            b"HTTP/1.1 302 Found\r\nLocation: http://login\r\n\r\n"
        ));
        assert!(is_portal(b"HTTP/1.1 204 No Content\r\n\r\nsurprise"));
        assert!(is_portal(b"garbage"));
    }

    fn serve(reply: &'static [u8]) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = c.read(&mut buf).unwrap();
                req.extend_from_slice(&buf[..n]);
            }
            c.write_all(reply).unwrap();
        });
        format!("http://127.0.0.1:{port}/generate_204")
    }

    #[test]
    fn gets_over_plain_http() {
        let t = Duration::from_secs(2);
        let open = serve(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(get(&open, t).map(|r| is_portal(&r)), Some(false));
        let portal = serve(b"HTTP/1.1 200 OK\r\n\r\n<form>");
        assert_eq!(get(&portal, t).map(|r| is_portal(&r)), Some(true));
    }

    #[test]
    fn no_answer_is_unknown() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        drop(l);
        assert_eq!(get(&url, Duration::from_millis(500)), None);
    }
}
