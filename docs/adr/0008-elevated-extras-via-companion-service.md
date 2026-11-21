# Elevated Extras come from an optional LocalSystem companion service, not an elevated app

Superseded in part: ADR 0002's "detected when the app is already elevated". The UI never runs elevated.

An installer running as admin cannot make later launches elevated, and Administrator protection makes every elevation interactive ([research 04](../research/04-elevated-extras.md)). So:

- **The Elevated Extras are bytes per process and UDP Hop Traces.** An optional per-machine service reads the Microsoft-Windows-Kernel-Network ETW provider while the Panel is open, runs UDP Hop Traces over raw sockets, and serves both over a local named pipe whose ACL admits interactive users only.
- **Account: LocalSystem.** Raw sockets need the Administrators group, which LocalService lacks. Research 04 ranked LocalService only on the condition that UDP Hop Traces were dropped; with two Extras needing the service, the user chose to keep them.
- ICMP and TCP Hop Traces stay in the app, unprivileged (ADR 0006).
- **Distribution:** a separate MSI, deployable by Intune or winget. Without it the app is unchanged.

## Considered Options

- **Scheduled task with "Run with highest privileges".** Rejected: it is a standing UAC bypass, and runs an egui UI as admin.
- **`requireAdministrator` manifest.** Rejected: a prompt at every launch, and fails ADR 0002 for standard users.
- **Elevated helper launched on demand.** Kept as a fallback for unmanaged machines.
- **Performance Log Users group.** Usable only where an organization already grants it.
