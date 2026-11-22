# Pauses: suspend, idle rest, kills and missed sleeps

Spec 5.0.0 makes time not measured a Pause, never loss, and drops any Round a Pause cuts (`rules/history.md`, Pauses). Windows records one in each of these cases:

- **Suspend and resume.** `RegisterSuspendResumeNotification` with a callback sends `Command::Suspend` and `Command::Resume` to the engine. The engine needs no window and no UI thread. Registering also opts the app into Modern Standby's suspend notice. The Pause ends on `PBT_APMRESUMEAUTOMATIC`, which Windows sends on every resume (research 07 §1).
- **Idle rest.** Display off, lock or screen saver already stopped the Prober. Now that rest is also a Pause. The `pause_when_idle` Setting keeps its default, on, so the app behaves as before.
- **Shutdown.** Quit begins a Pause, and the next launch ends it.
- **Kill.** The next launch opens a Pause from the second after the newest Raw result on disk.
- **A sleep with no notice,** such as a critical suspend. Every engine tick reads `QueryInterruptTime − QueryUnbiasedInterruptTime`, the time asleep since boot. A jump of more than 1 s becomes a Pause from the previous tick. A resume that the app was told about resets the reading, so the same sleep is not counted twice.
- **Cut Rounds.** The Prober sends each Round whole, with its start and end, and the engine drops a Round that `history::pause::cut` says a Pause touched.

**The settle time is 2 s**, set in `defaults.override.json` `history.settle_s`. It matches the spec vectors' example and is not yet measured on Windows. Measure the time from resume to first reply on Ethernet, Wi-Fi and a VPN, then update this ADR and the override. The Pause ends at a fixed time after resume, not when the network signals it is back, because the spec defines it that way.

**Meters draw one column a second,** so a Pause shows as empty columns. The Latency Meter ends a timeout plus a second before now, because the newest Rounds may still be waiting for replies.
