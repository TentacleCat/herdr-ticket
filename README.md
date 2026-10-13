# herdr-ticket

A Rust/ratatui ticket board for [Herdr](https://herdr.dev), backed by
[`wedow/ticket`](https://github.com/wedow/ticket). It reads the project's
`.tickets/*.md` files and uses `tk` for updates, so Markdown files remain the
source of truth and changes stay git-native.

## Features

- Kanban, List, and Table views.
- Move tickets between `open`, `in_progress`, and `closed` with `v` then `h`/`l`.
- Create, close, and reopen tickets through the `tk` command.
- Search by ID, title, status, type, assignee, or tag.
- Ticket details include priority, dependencies, tags, assignee, and description.
- Floating popup and split sidebar entrypoints.
- Reads the active Herdr workspace context, with `--root` and `TICKETS_DIR` overrides.

## Requirements

- Herdr 0.7.5 or newer.
- A recent stable Rust toolchain.
- [`wedow/ticket`](https://github.com/wedow/ticket) installed as `tk` and available on `PATH`.

The ticket CLI locates `.tickets/` from the project root. Herdr plugin panes run
from the plugin directory, so this board resolves the workspace paths supplied
by Herdr before falling back to the current directory. For manual use, pass
`--root /path/to/project` or set `TICKETS_DIR=/path/to/project/.tickets`.

## Install

```sh
herdr plugin install TentacleCat/herdr-ticket
```

The command palette will show **Tickets: open board** and **Tickets: open
sidebar**. Herdr plugin actions do not define global keybindings in the plugin
manifest, so add these recommended defaults to `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+shift+t"
type = "plugin_action"
command = "herdr-ticket.open-board"
description = "Open ticket board"

[[keys.command]]
key = "prefix+shift+b"
type = "plugin_action"
command = "herdr-ticket.open-sidebar"
description = "Open ticket sidebar"
```

Reload the Herdr config after changing the keybindings. If either key is
already assigned in your config, reassign or remove that binding first.

## Use from a checkout

```sh
git clone https://github.com/TentacleCat/herdr-ticket
cd herdr-ticket
cargo build --release
herdr plugin link .
```

## Keys

| Key | Action |
| --- | --- |
| `K` / `Tab` | Cycle Kanban, List, and Table |
| `j` / `k` | Move selection |
| `h` / `l` | Change Kanban column |
| `v`, then `h` / `l` | Move the selected ticket to an adjacent status |
| `a` | Create a ticket |
| `x` / `o` | Close / reopen the selected ticket |
| `s` | Cycle open, in progress, and closed |
| `Enter` / `d` | Toggle ticket details |
| `/` | Filter tickets |
| `C` | Show or hide closed tickets |
| `r` | Reload from `.tickets/` |
| `?` | Show help |
| `q` | Close the board |

Ticket status changes and creation run as argv-based `tk` subprocesses. The
board does not maintain a second issue database. Closed tickets are hidden by
default; press `C` to show them again.

## Local development

```sh
cargo build --release
herdr plugin link .
```

Run a plain text board preview without starting the TUI:

```sh
cargo run -- --root /path/to/project --selftest
```

MIT licensed. See [LICENSE](LICENSE).
