# Public Address: UDP for DNS, WinHTTP for HTTPS

Spec `rules/public-address.md` asks for each IP version's Public Address from two lookups: a TXT query to `ns3.cloudflare.com`, and an HTTPS GET of icanhazip.

- **DNS** is a hand-built TXT query over UDP to that IP version's address of the server (`probe::dns::txt_answers`), so it needs no resolver library and runs in Linux tests.
- **HTTPS** goes through WinHTTP, not a Rust TLS stack. Windows supplies TLS, the certificate store and the user's proxy settings, and the exe gains no TLS library (ADR 0007). Off Windows there is no HTTPS lookup, and the DNS answer is read as single-source.
- **When:** a thread looks both up at start, every `refresh_s` (300 s), and soon after the Links change. The engine copies the reading to the Panel every 10 s.
- **Shown, never judged:** the Network tab has a Public Address card per IP version. A DNS/HTTPS mismatch is flagged, but it doesn't reach the Verdict, as the spec says.
