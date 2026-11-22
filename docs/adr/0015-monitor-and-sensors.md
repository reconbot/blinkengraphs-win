# The engine is a Monitor over a Sensors seam

The engine loop called the Collector, `probe::path_up()` and `verdict::from_history` directly. Callers flipped a public `AtomicBool` to say the Panel was open. The crate had no tests, and the Verdict's inputs were assembled in a function nobody tested, with no way for new network facts such as a Captive Portal to reach them.

- `Sensors` is the seam where the Monitor reads the machine: Sample, processes, adapters, and `Network` (`path_up`, `via_vpn`, `captive_portal`). `SystemSensors` is the OS adapter; the tests use a scripted one. New network checks land in `Sensors::network`.
- `Monitor::tick(now, at)` holds what the loop did once per pass: record Probe results; when a Sample is due, pause or sample, read processes only while the Panel is open (ADR 0003), and judge. The thread only feeds it commands and the clock, so tests drive it with fake time.
- `engine::diagnose(history, network)` assembles the Verdict input and replaces `verdict::from_history`, so `verdict` no longer depends on `history`.
- The Panel being open is `Command::PanelOpen`, not shared state.
- `Shared` stays a mutex because the Panel reads History in place. Copying History into a snapshot every Sample would cost more than the lock does.
