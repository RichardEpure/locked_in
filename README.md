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
usage_page = 65376
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

At startup, LockedIn resolves one data root for `config.toml`, logs, panic logs, and
WebView data. Release builds use `%LOCALAPPDATA%\LockedIn`; debug builds use the
working directory. Set `LOCKED_IN_DATA_DIR` to use an isolated root for development
or automation.

Configuration loading is strict: the file must use schema version 2 and may not
contain unknown fields. A load failure shows an error screen and leaves the source
file unchanged. Editor changes remain local until Save; Save and explicit Reload
validate before atomically publishing one durable configuration to the UI and
automation runtime.

Close to tray is effective only when the tray icon was created successfully. Focused
window automation converges on the latest observed window at the next safe HID batch
boundary, ahead of queued Test and refresh commands; an in-flight report batch is
never interrupted.

# Setup
- Install Rust and the platform dependencies from the [Dioxus setup guide](https://dioxuslabs.com/learn/0.7/getting_started/).
- Install the pinned Dioxus CLI: `cargo install dioxus-cli --version 0.7.10 --locked`.
- Clone this repository.
- Install the locked Node dependencies: `npm ci`.

## Build
Run `npm run bundle` to generate compressed CSS and create a locked release desktop bundle.

## Develop
Run `npm run dev` to generate development CSS, watch Sass imports, and serve the desktop app.

## Test
Run `npm test` to generate development CSS and run the locked Rust test suite.
Repository verification also uses `cargo fmt --all --check`,
`cargo clippy --locked --all-targets -- -D warnings`, `npm run bundle`, and
`git diff --check`.
