# Ship one dual-mode WiX MSI; system installs add the companion service

One MSI with `Scope="perUserOrMachine"` ([research 05](../research/05-signing-and-installer.md)), installable two ways:

| | Per-user | Per-machine |
|---|---|---|
| Admin | Not needed | Needed |
| Location | `%LOCALAPPDATA%` | Program Files |
| Autostart | `Run` key in HKCU | `Run` key in HKLM |
| Companion service (M4, ADR 0008) | Not installed | Installed and started |
| Interactive | Setup asks "Just me" or "Everyone" | |
| Command line | `msiexec /i blinkengraphs.msi MSIINSTALLPERUSER=1 /qn` | `msiexec /i blinkengraphs.msi ALLUSERS=1 /qn` |
| Intune Win32 app | User context | System context |

- **One package:** the service is a component conditioned on a per-machine install, so per-user installs never need admin and Intune deploys one package either way.
- **Releases:** the MSI ships next to the bare exes. Its upgrade code makes Intune detection and upgrades work. Signing is deferred (ADR 0009).

## Considered Options

- **Separate MSI for the service.** Rejected: two packages to deploy and keep in step, for no gain once the service is conditional.
- **MSIX.** Its startup task only registers after the first launch, a packaged app cannot install the service, and Store policy 10.2.8 puts re-parenting into the taskbar at risk.
- **Inno Setup.** Works with Intune as a Win32 app, but an MSI gives Intune its native detection, and `msiexec` gives admins standard switches.
- **winget portable.** Fine as an extra channel later; it has no autostart and no Intune-native detection.
