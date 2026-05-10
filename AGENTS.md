# AGENTS.md

## Project Overview

Cross-platform terminal music player that streams audio files from GitHub repositories. Rust CLI + TUI application using tokio async runtime.

## Essential Commands

### Development Workflow
```bash
cargo build --release          # Build for release
cargo run --release --bin musictui2 -- tui # Run TUI in release mode
cargo test                  # Run tests
cargo fmt                   # Format code
cargo clippy -- -D warnings # Lint (strict)
```

### CLI Operations
```bash
cargo run --bin musictui2 -- add <owner>/<repo>      # Add repository (e.g., torvalds/linux)
cargo run --bin musictui2 -- list                   # List added repositories
cargo run --bin musictui2 -- remove <repo-id>       # Remove repository by ID or name
cargo run --bin musictui2 -- scan <owner>/<repo>     # Scan repository for audio files
cargo run --bin musictui2 -- list-tracks            # List all tracks
cargo run --bin musictui2 -- download <track-id>     # Download/cache a track
cargo run --bin musictui2 -- tui                    # Launch terminal UI
```

## Critical Dependencies

- **ratatui 0.28** - Terminal UI framework
- **rodio 0.19** - Cross-platform audio playback
- **rusqlite 0.32** - SQLite database (bundled)
- **reqwest 0.12** - HTTP client for GitHub API
- **tokio 1.40** - Async runtime
- **clap 4.5** - CLI argument parsing

## GitHub API Requirements

**Set `GITHUB_TOKEN` environment variable** for:
- Rate limit increase (5000 req/hour vs 60 anonymous)
- Access to private repositories

```bash
export GITHUB_TOKEN=ghp_your_token_here
```

Without token, you'll hit 60 requests/hour limit and get 403 Forbidden errors.

## Data Storage

- **Database**: `<config_dir>/musictui2/music.db` (SQLite)
- **Cache**: `<config_dir>/musictui2/cache/` (SHA256-keyed files, 1GB default)
- **Platform config dirs**: macOS: `~/Library/Application Support/`, Linux: `~/.config/`, Windows: `%APPDATA%`

## Project Structure

```
src/
├── main.rs          # CLI entry point (clap subcommands)
├── models.rs        # Repository, Track, PlaybackState
├── cli/mod.rs       # CLI operations (add/list/remove/scan)
├── tui/mod.rs       # Terminal UI (3 tabs: Repositories/Tracks/Now Playing)
├── github/mod.rs    # GitHub API client
├── database/mod.rs  # SQLite operations
├── cache/mod.rs     # File caching with LRU eviction
├── audio/mod.rs     # Rodio audio playback
└── events/mod.rs    # EventBus for cross-module comms
```

## Testing

- Unit tests in `tests/` directory
- Test file caching and model validation
- Coverage reporting via codecov in CI

## CI/CD

- **Testing**: ubuntu-latest, windows-latest, macos-latest
- **Rust versions**: stable, beta
- **Artifacts**: Build binaries uploaded for each platform
- **Coverage**: Codecov integration

## Audio Support

Supported formats: mp3, flac, wav, ogg, m4a, aac, wma

## TUI Controls

- `Tab` / `Shift+Tab` or `n` / `p` switch tabs
- `Up` / `Down` or `k` / `j` move within the Tracks tab
- `PageUp` / `PageDown` page through the Tracks tab
- `Enter` plays the selected track
- `Space` toggles play/pause
- `+` / `-` adjust volume
- `q` quits

## Key Implementation Notes

- **CLI first, TUI second**: All operations work via CLI before TUI
- **Event-driven**: Uses EventBus (mpsc) for cross-module communication
- **Cache-first**: Downloads files to cache before playback
- **Async throughout**: All I/O operations use tokio
- **Strict linting**: `cargo clippy -- -D warnings` enforced in CI
