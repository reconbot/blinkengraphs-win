# TCP Hop Traces are opt-in and experimental; ICMP Hop Traces are the default

The M0 spike (`spikes/tcp-hop`) ran on GitHub's `windows-latest` runner. The runner's network drops inbound ICMP:
- ICMP got no reply at any TTL.
- TCP connects with TTL 1–11 timed out instead of failing with `WSAEHOSTUNREACH`.
- TCP reached 1.1.1.1:443 from TTL 12.

That proves TTL-limited TCP connects work without admin. It cannot show whether `TCP_ICMP_ERROR_INFO` reports intermediate routers. Hop Traces therefore default to ICMP, which is proven without admin by WinMTR. A TCP Hop Trace can be added in config; a hop that yields no ICMP error info is shown as unknown. Run `tcp-hop.exe` on a real network to settle it.
