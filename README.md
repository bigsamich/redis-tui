# redis-tui

A terminal UI client for Redis inspired by Redis Insight, built with Rust and [ratatui](https://github.com/ratatui/ratatui).

## Features

- Browse keys across multiple Redis databases (0-9)
- View and edit values for all Redis data types: strings, hashes, lists, sets, sorted sets, and streams
- Filter keys with glob patterns
- Create, rename, and delete keys
- Set TTL on keys
- Binary data visualization with configurable data types and endianness
- Signal plot with zoom, pan, and auto-scaling
- FFT analysis (linear/log scale)
- Live stream listening via blocking XREAD
- Signal generator for writing waveform data to streams
- Mouse support for plot interaction (drag to pan, scroll to zoom)

## Installation

### From source

```bash
cargo build --release
```

The binary will be at `target/release/redis-tui`.

### Docker

```bash
docker build -t redis-tui .
docker run -it --rm redis-tui --host <redis-host>
```

To connect to Redis running on the host machine:

```bash
docker run -it --rm --network host redis-tui
```

## Usage

```
redis-tui [OPTIONS]
```

### Options

| Flag | Description | Default |
|------|-------------|---------|
| `--host <HOST>` | Redis host | `127.0.0.1` |
| `-p, --port <PORT>` | Redis port | `6379` |
| `--password <PASSWORD>` | Redis password | None |
| `-d, --db <DB>` | Redis database number | `0` |
| `-u, --url <URL>` | Full Redis URL (overrides other options) | None |

### Examples

```bash
# Connect to localhost
redis-tui

# Connect to a remote host
redis-tui --host 10.0.0.5 --port 6380

# Connect with a password
redis-tui --host myredis --password secret

# Connect with a full URL
redis-tui --url redis://:password@host:6379/2
```

## Keybindings

### Navigation

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Cycle between panels (Key List, Value View, Data Plot) |
| `Up` / `Down` | Navigate keys, scroll values, or switch between Signal/FFT plots |
| `Enter` | Load selected key's value |
| `0-9` | Switch Redis database |

### Key Operations

| Key | Action |
|-----|--------|
| `/` | Filter keys by glob pattern |
| `r` | Refresh key list |
| `s` | Edit selected key's value |
| `n` | Create new key |
| `d` | Delete selected key (with confirmation) |
| `z` | Set TTL on selected key |
| `R` | Rename selected key |
| `p` | Show/hide the plot panel |
| `?` | Show help |
| `q` / `Esc` | Quit |

### Data Plot

| Key | Action |
|-----|--------|
| `t` / `T` | Cycle data type forward/backward (Int8..Float64, String, Blob) |
| `e` | Toggle endianness (little/big) |
| `a` | Auto-fit plot limits |
| `x` | Set manual X-axis limits |
| `y` | Set manual Y-axis limits |
| `f` | Toggle FFT frequency analysis (split view) |
| `g` | Toggle FFT Y-axis scale (linear/log) |
| Mouse drag | Pan |
| Mouse scroll | Zoom |

### Streams

| Key | Action |
|-----|--------|
| `l` | Start/stop live stream listener (XREAD) |
| `w` | Open signal generator / stop running generator |

### Edit Mode

| Key | Action |
|-----|--------|
| `Ctrl+B` | Toggle binary encoding mode |
| `Ctrl+T` | Cycle binary data type |
| `Ctrl+E` | Toggle endianness |
| `Tab` / `Shift+Tab` | Navigate between fields |
| `Enter` | Submit/apply changes |
| `Esc` | Cancel/close popup |
