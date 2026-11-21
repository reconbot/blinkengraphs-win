# Elevated Extras come from an optional companion service, not an elevated app

Superseded in part: ADR 0002's "detected when the app is already elevated". The UI never runs elevated.

An installer running as admin cannot make later launches elevated, and Administrator protection makes every elevation interactive ([research 04](../research/04-elevated-extras.md)). So:

- **UDP Hop Traces are dropped.** They were the only feature needing raw sockets. ICMP traces need no admin; TCP traces use `TCP_ICMP_ERROR_INFO` (ADR 0006). UDP remains a latency Probe.
- **Bytes per process is the one Elevated Extra.** An optional per-machine service reads the Microsoft-Windows-Kernel-Network ETW provider only while the Panel is open and serves totals over a local named pipe whose ACL admits interactive users only.
- **Account: LocalService if a spike shows it can enable that provider, else LocalSystem.** No Microsoft doc settles it; Velociraptor implies SYSTEM.
- **Distribution:** a separate MSI, deployable by Intune or winget. Without it the app is unchanged.

## Considered Options

- **Scheduled task with "Run with highest privileges".** Rejected: it is a standing UAC bypass, and runs an egui UI as admin.
- **`requireAdministrator` manifest.** Rejected: a prompt at every launch, and fails ADR 0002 for standard users.
- **Elevated helper launched on demand.** Kept as a fallback for unmanaged machines.
- **Performance Log Users group.** Usable only where an organization already grants it.
