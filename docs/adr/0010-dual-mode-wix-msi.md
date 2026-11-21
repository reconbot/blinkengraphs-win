# Ship a dual-mode WiX MSI

One MSI with `Scope="perUserOrMachine"` ([research 05](../research/05-signing-and-installer.md)):
- **Per-user, no admin:** installs under `%LOCALAPPDATA%` for a user double-clicking it, or for Intune's Win32 app in user context.
- **Per-machine:** installs under Program Files for Intune's system context or `msiexec ALLUSERS=1`.
- **Autostart:** the `Run` key in the matching hive.
- **Releases:** the MSI ships next to the bare exes, signed like them (ADR 0009). Its product code and upgrade code make Intune detection and upgrades work.
- **The M4 companion service** ships as a separate per-machine MSI (ADR 0008).

## Considered Options

- **MSIX.** Its startup task only registers after the first launch, a packaged app cannot install the M4 service, and Store policy 10.2.8 puts re-parenting into the taskbar at risk.
- **Inno Setup.** Works with Intune as a Win32 app, but an MSI gives Intune its native detection, and `msiexec` gives admins standard switches.
- **winget portable.** Fine as an extra channel later; it has no autostart and no Intune-native detection.
