# cronv

A modern, keyboard-driven terminal UI for managing cron jobs — with human-readable
schedule descriptions, next-run times, a 24-hour firing timeline, and a live preview editor.

```
  cronv  ·  12 jobs  ·  system crontab  [24h]

 ● │ Schedule        │ Description                          │ Next Run              │ Command
───┼─────────────────┼──────────────────────────────────────┼───────────────────────┼────────────────────
▶● │ */15 * * * *    │ Every 15 minutes                     │ 09:15 (in 13m)        │ run-parts /etc/...
 ● │ 0 2 * * *       │ Daily at 02:00                       │ Tue 02:00 (in 5h)     │ run-parts /etc/...
 ● │ 0 3 * * 6       │ Saturdays at 03:00                   │ Sat 03:00 (in 4 days) │ run-parts /etc/...
 ● │ 0 4 * * 0,3     │ Sundays and Wednesdays at 04:00      │ Wed 04:00 (in 1 day)  │ /usr/bin/pihole…
 ● │ 5 4 1-7 * 0     │ First Sunday of the month at 04:05   │ Sun 04:05 (in 5 days) │ /usr/bin/apt up…
 ○ │ */5 * * * *     │ Every 5 minutes                      │ disabled              │ curl -s https://…
```

## Features

- **Smart descriptions** — `*/5 9,12 1 2-4 *` → _Every 5 minutes from 09:00 to 09:55 and 12:00 to 12:55, on the 1st in February through April_
- **Next-run column** — live calculation of the next fire time with relative label (`in 13m`, `in 4 days`)
- **Job info panel** — press `i` to see the next 10 runs and a 24-hour firing-pattern chart
- **Live preview editor** — field-level validation with per-field allowed-value hints
- **@special keywords** — `@reboot`, `@daily`, `@hourly`, etc. with autocomplete panel
- **12h/24h toggle** — switch clock format with `c`
- **File mode** — edit any crontab file directly with `--file`
- **Strict validation** — invalid field values are never shown as entries

## Installation

### Pre-built binaries

Download the latest release for your platform from the
[Releases page](https://github.com/you/cronv/releases):

| Platform | Binary |
|---|---|
| Linux x86-64 (glibc) | `cronv-linux-x86_64.tar.gz` |
| Linux x86-64 (musl / Alpine) | `cronv-linux-x86_64-musl.tar.gz` |
| macOS Intel | `cronv-macos-x86_64.tar.gz` |
| macOS Apple Silicon | `cronv-macos-aarch64.tar.gz` |

```bash
# Example: Linux
curl -LO https://github.com/you/cronv/releases/latest/download/cronv-linux-x86_64-musl.tar.gz
tar xzf cronv-linux-x86_64-musl.tar.gz
sudo mv cronv /usr/local/bin/
```

### Alpine Linux / Docker

The musl binary is fully static — no dependencies required:

```dockerfile
FROM alpine:latest
COPY cronv /usr/local/bin/cronv
```

### Build from source

Requires Rust 1.75+ (stable).

```bash
git clone https://github.com/you/cronv
cd cronv
cargo build --release
# Binary: target/release/cronv
```

#### Static musl binary (Alpine-compatible)

```bash
rustup target add x86_64-unknown-linux-musl
sudo apt-get install musl-tools   # Debian/Ubuntu
cargo build --release --target x86_64-unknown-linux-musl
# Binary: target/x86_64-unknown-linux-musl/release/cronv
```

## Usage

```
cronv [OPTIONS]

Options:
  -f, --file <PATH>   Edit a crontab file directly instead of the system crontab
  -V, --version       Print version and exit
  -h, --help          Print this help and exit
```

```bash
cronv                        # Edit the current user's system crontab
cronv --file /etc/crontab    # Edit a system-wide crontab file
cronv -f ~/my-jobs.cron      # Edit any file as a crontab
```

## Key bindings

### Main list

| Key | Action |
|---|---|
| `↑` / `k` | Move up |
| `↓` / `j` | Move down |
| `n` / `a` | Add new cron job |
| `e` / `Enter` | Edit selected entry |
| `i` | Job info: next 10 runs + 24h chart |
| `d` | Delete selected entry (with confirmation) |
| `t` | Toggle enable / disable (comments the line) |
| `s` | Save crontab to system |
| `c` | Toggle 12h / 24h clock display |
| `?` | Help overlay |
| `q` / `Esc` | Quit (prompts if there are unsaved changes) |

### Editor

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Next / previous field |
| `F1` | Toggle between `@special` and 5-field mode |
| `Ctrl+S` | Save entry |
| `Enter` | Advance to next field, or save on Command field |
| `Esc` | Cancel edit |

## Schedule syntax

```
┌───── minute       (0–59)     */5  0,15,30,45  10-20
│ ┌─── hour         (0–23)     */2  9,17  8-18
│ │ ┌─ day-of-month (1–31)     */5  1,15  1-7   L
│ │ │ ┌ month       (1–12)     */3  2-4   1,6,12
│ │ │ │ ┌ weekday   (0–7)      1-5  0,6   MON-FRI  (0/7 = Sunday)
* * * * *
```

### @special shortcuts

| Keyword | Equivalent | Description |
|---|---|---|
| `@reboot` | — | At system startup |
| `@hourly` | `0 * * * *` | Every hour at :00 |
| `@daily` / `@midnight` | `0 0 * * *` | Daily at midnight |
| `@weekly` | `0 0 * * 0` | Sundays at midnight |
| `@monthly` | `0 0 1 * *` | 1st of month at midnight |
| `@yearly` / `@annually` | `0 0 1 1 *` | Jan 1 at midnight |

## Description examples

| Schedule | Description |
|---|---|
| `*/15 * * * *` | Every 15 minutes |
| `0 * * * *` | Every hour |
| `0 9 * * 1-5` | Weekdays at 09:00 |
| `30 2 * * 5` | Fridays at 02:30 |
| `0 4 * * 0,3` | Sundays and Wednesdays at 04:00 |
| `5 4 1-7 * 0` | First Sunday of the month at 04:05 |
| `0 4,5 * * *` | Daily at 04:00 and 05:00 |
| `*/5 9,12 1 2-4 *` | Every 5 minutes from 09:00 to 09:55 and 12:00 to 12:55, on the 1st in February through April |

## License

MIT
