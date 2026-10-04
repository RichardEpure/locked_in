# About

LockedIn is a companion app where you can create rules that listen and trigger on system events and send reports to peripherals that can listen for raw HID events (e.g. [QMK](https://docs.qmk.fm/features/rawhid)), allowing your peripherals to react to system events.

An automation consists of:

- One event.
- Ordered cases; the first matching case runs.
- Optional exception matchers for each case.
- One or more raw HID report actions routed to reusable devices.
- An optional `otherwise` branch when no case matches.

## Platform Support

- [x] Windows
- [ ] macOS
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
id = "device"
name = "My Device"
vid = 45752
pid = 0
usage_page = 65376
usage = 80
report_length = 32
report_id = 0

[[automations]]
id = "automation"
name = "Application layers"
enabled = true
event = "focused_window_changed"

[[automations.cases]]
id = "case-1"
name = "Game"

# Fields in one matcher are ANDed. Separate matchers are ORed.
[[automations.cases.applications]]
id = "matcher-1"
title = { operator = "contains", value = "Game", case_sensitive = false }
exe = { operator = "equals", value = 'C:\Games\Game.exe', case_sensitive = false }

# Skip launcher windows from the same executable and continue evaluation.
[[automations.cases.exceptions]]
id = "matcher-2"
title = { operator = "contains", value = "Launcher", case_sensitive = false }

[[automations.cases.actions]]
id = "action-1"
label = "Switch to gaming layer"
report = [135]
device_ids = ["device"]

[[automations.otherwise_actions]]
id = "action-2"
label = "Switch to base layer"
report = [134]
device_ids = ["device"]
```

Matcher operators are `equals`, `contains`, and `regex`. Reports shorter than a device's
configured report length are zero-padded; oversized reports are rejected before save or test.

`report` contains payload bytes only, and `report_length` is the payload capacity,
excluding the report-ID byte. LockedIn pads the payload and prepends `report_id` when
writing through HIDAPI. The device selectors, report length, report ID, and payloads
must match your peripheral's firmware. The example bytes `[135]` and `[134]` only
does something if your firmware listens for them; the action labels do not
give them built-in meaning.

At startup, LockedIn resolves one data root for `config.toml`, logs, panic logs, and
WebView data. Release builds use `%LOCALAPPDATA%\LockedIn`; debug builds use the
working directory. Set `LOCKED_IN_DATA_DIR` to use an isolated root for development
or automation.

# Setup

The tool versions below match the Windows CI toolchain:

- Install Node.js 24, which includes npm.
- Install and select Rust 1.98.0 with the rustfmt and Clippy components.
- Install Dioxus CLI 0.7.10: `cargo install dioxus-cli --version 0.7.10 --locked`.
- Install the Windows desktop platform dependencies from the [Dioxus setup guide](https://dioxuslabs.com/learn/0.7/getting_started/).
- Clone this repository.
- Install the locked Node dependencies: `npm ci`.

## Build

Run `npm run bundle` to generate compressed CSS and create a locked release desktop bundle.

## Develop

Run `npm run dev` to generate development CSS, watch Sass imports, and serve the desktop app.

## Test

Run `npm test` to generate required assets and run the Rust test suite.

## Verify

Run `npm run verify` to check formatting, run tests and Clippy, and build the release
bundle.
