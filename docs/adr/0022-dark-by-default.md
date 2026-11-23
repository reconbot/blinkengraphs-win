# Dark by default

The spec's default Theme is System (`defaults.json` `appearance`). The owner wants Windows dark from the first launch, so `defaults.override.json` sets `appearance` to `dark` (spec `rules/defaults.md`).

A config saved before this keeps the Theme it saved, System included. The Settings tab still offers System, Light and Dark.

The Theme now colors the Panel only; the taskbar Surfaces follow the taskbar by default (ADR 0023).
