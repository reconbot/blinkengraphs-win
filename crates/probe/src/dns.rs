//! Minimal DNS wire format (RFC 1035 §4.1): A queries for the DNS Probe, TXT for the Public
//! Address.

const FLAG_QR: u16 = 0x8000;
const FLAG_RD: u16 = 0x0100;
const TYPE_A: u16 = 1;
pub const TYPE_TXT: u16 = 16;
const CLASS_IN: u16 = 1;

pub fn encode_query(id: u16, name: &str) -> Vec<u8> {
    encode_query_type(id, name, TYPE_A)
}

pub fn encode_query_type(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + name.len() + 6);
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&FLAG_RD.to_be_bytes());
    out.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out.extend_from_slice(&qtype.to_be_bytes());
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

/// The strings of every TXT record in the response to query `id`, or `None` unless `buf` is a
/// whole, successful response to it.
pub fn txt_answers(buf: &[u8], id: u16) -> Option<Vec<String>> {
    let h = parse_response(buf)?;
    if h.id != id || h.rcode != 0 {
        return None;
    }
    let word =
        |i: usize| -> Option<u16> { Some(u16::from_be_bytes([*buf.get(i)?, *buf.get(i + 1)?])) };
    let questions = word(4)?;
    let mut i = 12;
    for _ in 0..questions {
        i = skip_name(buf, i)? + 4;
    }
    let mut out = Vec::new();
    for _ in 0..h.answers {
        i = skip_name(buf, i)?;
        let (rtype, len) = (word(i)?, word(i + 8)? as usize);
        let data = buf.get(i + 10..i + 10 + len)?;
        i += 10 + len;
        if rtype != TYPE_TXT {
            continue;
        }
        // RDATA is one or more length-prefixed strings.
        let mut j = 0;
        while j < data.len() {
            let n = data[j] as usize;
            let s = data.get(j + 1..j + 1 + n)?;
            out.push(String::from_utf8_lossy(s).into_owned());
            j += 1 + n;
        }
    }
    Some(out)
}

/// The offset just past the name at `i`: labels to a zero byte, or a two-byte pointer.
fn skip_name(buf: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let len = *buf.get(i)?;
        match len {
            0 => return Some(i + 1),
            l if l & 0xc0 == 0xc0 => return Some(i + 2),
            l => i += 1 + l as usize,
        }
    }
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

    /// A response to a TXT query with two TXT answers, their names compressed to the question.
    fn txt_response(id: u16, strings: &[&str]) -> Vec<u8> {
        let mut r = encode_query_type(id, "whoami.cloudflare.com", TYPE_TXT);
        r[2] |= 0x80;
        r[7] = strings.len() as u8;
        for s in strings {
            r.extend_from_slice(&[0xc0, 12]);
            r.extend_from_slice(&TYPE_TXT.to_be_bytes());
            r.extend_from_slice(&[0, 1, 0, 0, 0, 0]);
            r.extend_from_slice(&((s.len() + 1) as u16).to_be_bytes());
            r.push(s.len() as u8);
            r.extend_from_slice(s.as_bytes());
        }
        r
    }

    #[test]
    fn reads_the_txt_strings_of_an_answer() {
        let r = txt_response(9, &["remote_ip: 198.51.100.7", "asn: 64500"]);
        assert_eq!(
            txt_answers(&r, 9),
            Some(vec!["remote_ip: 198.51.100.7".into(), "asn: 64500".into()])
        );
        assert_eq!(txt_answers(&r, 10), None, "another query's answer");
        assert_eq!(txt_answers(&r[..r.len() - 3], 9), None, "cut short");
    }

    #[test]
    fn rejects_queries_and_short_packets() {
        assert_eq!(parse_response(&encode_query(7, QUERY_NAME)), None);
        assert_eq!(parse_response(&[0x12, 0x34, 0x80]), None);
    }
}
