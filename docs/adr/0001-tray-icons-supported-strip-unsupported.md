# Tray Icons are the supported Surface; the Taskbar Strip is unsupported and falls back

Windows 11 removed deskbands, and Microsoft says "Apps can no longer customize areas of the Taskbar" (docs/research/01-prior-art.md §5). Tray Icons use the documented `Shell_NotifyIcon` API and are the default Surface. The Taskbar Strip ships in v1 anyway, because wide graphs are worth it. It works by re-parenting a layered window into `Shell_TrayWnd`, as TrafficMonitor and StatsOverlay do. That is an undocumented technique, so it lives behind the same Surface interface as Tray Icons. Whenever it cannot attach or lay out, it falls back to Tray Icons automatically, and an Explorer restart or a Windows update must never leave the user with no Meters.

## Consequences

- The Taskbar Strip is expected to break on some Windows updates. Breakage is a fallback, not a crash.
- Meter drawing is independent of the Surface. One renderer produces images for Tray Icons, the Taskbar Strip and CI screenshots.
