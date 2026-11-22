//! Minimal DNS A-query wire format (RFC 1035 §4.1).

const FLAG_QR: u16 = 0x8000;
const FLAG_RD: u16 = 0x0100;
const TYPE_A: u16 = 1;
const CLASS_IN: u16 = 1;

pub fn encode_query(id: u16, name: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + name.len() + 6);
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&FLAG_RD.to_be_bytes());
    out.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out.extend_from_slice(&TYPE_A.to_be_bytes());
    out.extend_from_slice(&CLASS_IN.to_be_bytes());
    out
}

#[derive(Debug, PartialEq, Eq)]
pub struct Header {
    pub id: u16,
    pub rcode: u8,
    pub answers: u16,
}

/// `None` unless `buf` is a DNS response.
pub fn parse_response(buf: &[u8]) -> Option<Header> {
    let word = |i: usize| u16::from_be_bytes([buf[i], buf[i + 1]]);
    if buf.len() < 12 || word(2) & FLAG_QR == 0 {
        return None;
    }
    Some(Header {
        id: word(0),
        rcode: (word(2) & 0x000f) as u8,
        answers: word(6),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUERY_NAME: &str = "cloudflare.com";

    #[test]
    fn encodes_a_query_with_recursion_desired() {
        let q = encode_query(0xbeef, "cloudflare.com");
        let mut want = vec![0xbe, 0xef, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        want.push(10);
        want.extend_from_slice(b"cloudflare");
        want.push(3);
        want.extend_from_slice(b"com");
        want.extend_from_slice(&[0, 0, 1, 0, 1]);
        assert_eq!(q, want);
    }

    #[test]
    fn trailing_dot_is_ignored() {
        assert_eq!(encode_query(1, "a.b."), encode_query(1, "a.b"));
    }

    #[test]
    fn parses_a_response_header() {
        let mut r = encode_query(0x1234, QUERY_NAME);
        r[2] |= 0x80;
        r[3] = 0x83;
        r[7] = 2;
        assert_eq!(
            parse_response(&r),
            Some(Header {
                id: 0x1234,
                rcode: 3,
                answers: 2
            })
        );
    }

    #[test]
    fn rejects_queries_and_short_packets() {
        assert_eq!(parse_response(&encode_query(7, QUERY_NAME)), None);
        assert_eq!(parse_response(&[0x12, 0x34, 0x80]), None);
    }
}
