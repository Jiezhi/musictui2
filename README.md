# Musictui2

A cross-platform terminal music player that finds and streams audio files from GitHub repositories.

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.71+-orange.svg)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)

## Features

- GitHub repository scanning for `mp3`, `flac`, `wav`, `ogg`, `m4a`, `aac`, and `wma` files
- Terminal UI built with `ratatui`
- Cross-platform playback through `rodio`
- Local SQLite library for repositories and tracks
- SHA256-keyed file cache for downloaded audio
- TUI playback can stream uncached tracks while the cache is being written
- CLI-first workflow with an optional TUI

## Prerequisites

- Rust 1.71 or newer
- A GitHub token for practical scanning limits

Set the token in your shell or in a project-local `.env` file:

```bash
export GITHUB_TOKEN=your_github_token_here
```

Without a token, GitHub limits API access to 60 requests per hour. Authenticated requests allow 5000 requests per hour and can access private repositories when the token has the required permissions.

## Quick Start

```bash
git clone https://github.com/yourusername/musictui2.git
cd musictui2
cargo build --release
```

Run commands through the main binary:

```bash
cargo run --release --bin musictui2 -- <command>
```

Launch the TUI:

```bash
cargo run --release --bin musictui2 -- tui
```

## Usage

Add and scan a repository:

```bash
cargo run --release --bin musictui2 -- add owner/repo
cargo run --release --bin musictui2 -- scan owner/repo
```

List or remove repositories:

```bash
cargo run --release --bin musictui2 -- list
cargo run --release --bin musictui2 -- remove owner/repo
cargo run --release --bin musictui2 -- remove <repo-id>
```

Work with tracks:

```bash
cargo run --release --bin musictui2 -- list-tracks
cargo run --release --bin musictui2 -- list-tracks --repository owner/repo
cargo run --release --bin musictui2 -- download <track-id>
```

Refresh a repository scan:

```bash
cargo run --release --bin musictui2 -- update-scan owner/repo
```

After installing the release binary somewhere on your `PATH`, use the shorter form:

```bash
musictui2 tui
musictui2 add owner/repo
musictui2 list-tracks
```

## TUI Controls

- `Tab`: switch tabs
- `j` / `Down`: move down in the track list
- `k` / `Up`: move up in the track list
- `Enter`: play the selected track
- `Space`: play or pause
- `+` / `-`: adjust volume
- `q`: quit

The Tracks tab includes a `Cache` column. `Cached` means the local cache file is complete, `Caching` means the selected track is currently being written while playback starts, and `-` means it is not cached yet.

## Data Storage

Musictui2 stores its database and cache under the platform configuration directory:

- macOS: `~/Library/Application Support/musictui2/`
- Linux: `~/.config/musictui2/`
- Windows: `%APPDATA%\musictui2\`

The SQLite database is `music.db`. Downloaded audio is stored in `cache/`. Database schema migrations run automatically on startup.

## Development

```bash
cargo test
cargo fmt
cargo clippy -- -D warnings
```

Because this crate has multiple binaries, use `--bin musictui2` when running the application with Cargo:

```bash
cargo run --bin musictui2 -- --help
```

## Project Structure

```text
src/
├── main.rs          # CLI entry point
├── lib.rs           # Library exports
├── models.rs        # Repository, Track, PlaybackState
├── cli/mod.rs       # CLI operations
├── tui/mod.rs       # Terminal UI
├── github/mod.rs    # GitHub API client and scanner
├── database/mod.rs  # SQLite storage and migrations
├── cache/mod.rs     # File caching
├── audio/mod.rs     # Rodio playback
└── events/mod.rs    # Event bus
```

## Troubleshooting

If scanning fails with `403 Forbidden`, set `GITHUB_TOKEN` and retry. See [GITHUB_API_SETUP.md](GITHUB_API_SETUP.md) for token setup details.

If `cargo run` reports that it cannot determine which binary to run, include the binary name:

```bash
cargo run --bin musictui2 -- tui
```

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE) for details.

## Acknowledgments

- [ratatui](https://github.com/ratatui-org/ratatui)
- [rodio](https://github.com/RustAudio/rodio)
- [rusqlite](https://github.com/rusqlite/rusqlite)
- [clap](https://github.com/clap-rs/clap)
