# Footprint is a feature: budgets enforced in CI

A system monitor that costs noticeable CPU, memory or disk distorts what it measures, and it loses the trust of the people it is meant to help. Small size and low overhead are therefore requirements, not later polish.

Budgets for x64 with the default config and the Panel closed:
- **Executable:** at most 7 MiB. CI fails above this.
- **CPU:** under 1% of one core.
- **Working set:** under 40 MB.

`tools/footprint` measures the engine plus Meter and Strip drawing for 60 s on Windows CI and posts the result in the job summary.

To stay inside them, the release build uses `opt-level = "s"`, `panic = "abort"`, LTO and strip. That brought the exe from 9.4 MB to 6.6 MB, and dropping eframe's wgpu backend had already taken it from 16 MB to 9.4 MB. A new dependency has to justify its bytes.
