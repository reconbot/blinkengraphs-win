# CI screenshots come from headless rendering of fixtures, not desktop capture

GitHub-hosted Windows runners may not provide an interactive desktop session. That is reported in community threads, not documented by GitHub, so confidence is low. A screenshot pipeline that depends on capturing the real tray would therefore be flaky. Instead, every visual (Meters, Taskbar Strip, Panel) is drawn from recorded or synthetic fixture data by the same rendering code the app uses, written to PNGs by a `screenshots` binary that runs on Windows CI and locally on Linux. CI publishes the PNGs as follows:
- Commits them to an orphan `ci-screenshots` branch so they have stable URLs.
- Embeds them in a sticky PR comment.
- Embeds them in the job summary.

A real-desktop capture job may be added later, but only as a best-effort extra.
