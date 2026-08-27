# About
LockedIn is a companion app where you can create rules that listen and trigger on system events and send reports to perhipherals that can listen for raw hid events (e.g. [QMK](https://docs.qmk.fm/features/rawhid)), allowing your perhipherals to react to system events.

An automation consists of:
- One event.
- Ordered cases; the first matching case runs.
- Optional exception matchers for each case.
- One or more raw HID report actions routed to reusable devices.
- An optional `otherwise` branch when no case matches.

## Platform Support
- [x] Windows
- [ ] MacOS
- [ ] Linux

## Example `config.toml`
```toml
version = 2

[settings]
start_minimized = true
close_to_tray = true
start_with_windows = false
log_level = "info"

[[devices]]
id = "my-device"
name = "My Device"
vid = 45752
pid = 0
usage_page = 66012
usage = 80
report_length = 32
report_id = 0

[[automations]]
id = "application-layers"
name = "Application layers"
enabled = true
event = "focused_window_changed"

[[automations.cases]]
id = "game"
name = "Game"

# Fields in one matcher are ANDed. Separate matchers are ORed.
[[automations.cases.applications]]
id = "game-window"
title = { operator = "contains", value = "Game", case_sensitive = false }
exe = { operator = "equals", value = 'C:\Games\Game.exe', case_sensitive = false }

# Any matching exception skips this case and continues evaluation.
[[automations.cases.exceptions]]
id = "browser-false-positive"
exe = { operator = "equals", value = 'C:\Program Files\Browser\browser.exe', case_sensitive = false }

[[automations.cases.actions]]
id = "gaming-layer"
label = "Switch to gaming layer"
report = [135]
device_ids = ["my-device"]

[[automations.otherwise_actions]]
id = "base-layer"
label = "Switch to base layer"
report = [134]
device_ids = ["my-device"]
```

Matcher operators are `equals`, `contains`, and `regex`. Reports shorter than a device's
configured report length are zero-padded; oversized reports are rejected before save or test.

Release builds store `config.toml`, logs, panic logs, and WebView data under
`%LOCALAPPDATA%\LockedIn`. Debug builds use the repository directory. Set
`LOCKED_IN_DATA_DIR` to use an isolated data directory for development or automation.

# Setup
- This project uses [Dioxus](https://dioxuslabs.com/), make sure you go through the [setup here](https://dioxuslabs.com/learn/0.7/getting_started/).
- Clone this repo
- `npm install`
- `cargo install`
## Build
- `dx bundle`
## Develop
- `npm run css:watch`
- `dx serve`
