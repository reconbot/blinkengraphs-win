# Footprint is a feature

A system monitor that costs noticeable CPU, memory or disk distorts what it measures, and it loses the trust of the people it is meant to help. Small size and low overhead are therefore requirements, not later polish.

Targets for x64 with the default config and the Panel closed:
- **Executable:** about 7 MiB. This is a guideline that keeps the binary small, not a hard limit: a change that does the right thing may cost bytes, and its PR says how many. CI reports the size and raises a warning above 7 MiB so the cost gets evaluated; it never fails on it.
- **CPU:** under 1% of one core.
- **Working set:** under 40 MB.

`tools/footprint` measures the engine plus Meter and Strip drawing for 60 s on Windows CI and posts the result in the job summary.

To stay near them, the release build uses `opt-level = "s"`, `panic = "abort"`, LTO and strip. That brought the exe from 9.4 MB to 6.6 MB, and dropping eframe's wgpu backend had already taken it from 16 MB to 9.4 MB. A new dependency still has to justify its bytes.
