# Never require Administrator; Elevated Extras light up only when elevated

blinkengraphs must be useful to a standard user on a managed (Entra/Intune) machine, so every default feature uses only APIs that need no Administrator rights:
- `IcmpSendEcho2` for ICMP Probes and ICMP Hop Traces.
- Ordinary TCP `connect()` for TCP Probes.
- Ordinary UDP sockets for DNS Probes.
- PDH, `GetSystemTimes`, `GetIfTable2` and `NtQuerySystemInformation` for Metrics.

Raw sockets and ETW kernel sessions require Administrators (Microsoft Learn, "TCP/IP raw sockets"). Bytes per process on the network and UDP Hop Traces are therefore Elevated Extras: they are detected at startup and never requested through UAC.

## Considered Options

- **Raw-socket probing everywhere, like Trippy.** Rejected, because Trippy "must be run with Administrator privileges on Windows."
- **Shelling out to `ping.exe` or `tracert.exe`.** Rejected: it starts a process per Probe, the output is localized and must be parsed, and timing is coarse.

## Consequences

TCP Hop Traces depend on an unproven route. Setting `IP_TTL` on a TCP socket, enabling `TCP_FAIL_CONNECT_ON_ICMP_ERROR` and reading `WSAGetIcmpErrorInfo` (Windows 10 2004+) might yield TTL-exceeded hops without admin. A CI spike (PLAN M0) decides whether TCP Hop Traces are a default feature or an Elevated Extra.
