# The Prober's rules sit above a Transport seam

The Ticker mixed the spec's rules with Win32 calls behind `#[cfg(windows)]`. Off Windows, ICMP Probes and Hop Traces silently vanished, so rules like "an IP version without a route means absent, not lost" could only be tested on Windows.

- `probe::Transport` is everything the Prober asks of the network: family up, lookup, TCP, UDP and DNS round trips, an ICMP echo batch, and a TCP Hop Trace. `SystemTransport` is the OS adapter: IP Helper and sockets on Windows, sockets alone elsewhere, where `echo` and `tcp_trace` return `None`.
- The Ticker keeps the rules and holds no platform code: skip a Target whose IP version has no route; send nothing without any path; count an unresolved Target as lost; batch ICMP Probes with ICMP Hop Traces; remember path length.
- `Prober::start_with` takes a Transport, so `crates/probe/tests/rules.rs` runs those rules on Linux CI over a scripted network.
- One `Icmp` per Prober is shared by every group thread. It is `Sync` because the handles are kernel objects and each batch brings its own events and buffers.
- New lookups, such as Public Address, become new Transport calls rather than new `cfg` branches.
