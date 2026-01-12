# About
LockedIn is a companion app where you can create rules that listen and trigger on system events and send reports to perhipherals that can listen for raw hid events (e.g. [QMK](https://docs.qmk.fm/features/rawhid)), allowing your perhipherals to react to system events.

A rule consists of:
- An event.
- Specific configuration options for the given event.
- Various reports that get sent depending on set conditions.
- A list of hid devices to send reports to.

## Platform Support
- [x] Windows
- [ ] MacOS
- [ ] Linux

## Example `config.toml`
```toml
[[rules]]
name = "Example Rule"

[rules.event]
type = "focused_window_changed" # Event triggers the rule when the current focused window changes.
on_match_reports = [[135]] # If the newly focused window matches any in the inclusion list, send these reports.
on_no_match_reports = [[134]] # If the newly focused window matches any in the exclusion list or matches nothing, send these reports.

[[rules.event.inclusions]]
title = "WindowTitle"
class = "WindowClass"
exe = 'C:\Path\To\Executable.exe'

# Each property is optional, only a single one has to match.
[[rules.event.exclusions]]
title = "WindowTitleToExclude"

[[rules.devices]]
name = "MyDevice"
vid = 45752
pid = 0
usage_page = 66012
usage = 80
report_length = 32
report_id = 0
```

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
