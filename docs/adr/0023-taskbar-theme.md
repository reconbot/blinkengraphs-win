# The taskbar has its own Theme

Spec `CONTEXT.md` Appearance is one light or dark color set for every Surface and the Panel. Windows departs from it for its normal use (spec ADR 0002).

Windows lets the taskbar be light while apps are dark, and ADR 0022 makes the Panel dark by default. With one Appearance, a light taskbar got the dark set: pale fills and white text meant for a dark background.

So there are two Settings:
- **Theme** (`appearance.mode`) colors the Panel. The default is Dark (ADR 0022).
- **Taskbar theme** (`appearance.taskbar`) colors the Taskbar Strip, the Tray Icons and the Menu Icon. The default is System, which follows the taskbar's own shade and gives the spec's light palette on a light taskbar.

A config saved before this has no `taskbar` key and follows the taskbar.
