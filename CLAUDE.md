# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build --release                       # Release build
cargo run --bin musictui2 -- <subcommand>   # Run main CLI (must specify --bin; crate has multiple binaries)
cargo run --bin musictui2 -- tui            # Launch TUI

cargo test                                  # Run all tests
cargo test --test cache_test                # Run a specific integration test file in tests/
cargo test <name>                           # Run tests matching name (e.g. cargo test migrates_legacy)
cargo test -- --nocapture                   # Show println! output during tests

cargo fmt --all -- --check                  # CI formatting check
cargo clippy -- -D warnings                 # CI lint (warnings fail the build)
```

Linux builds require `libasound2-dev` and `pkg-config` (rodio/ALSA).

The repo defines auxiliary binaries under `src/bin/` (`debug_audio`, `test_audio`, `test_tui_audio`) — these are not the application, always use `--bin musictui2` to run the app.

## Environment

`GITHUB_TOKEN` is read from the environment or from a `.env` file loaded at startup (`main.rs::load_env_file`). The loader is a minimal `KEY=VALUE` line parser — no quoting, no comments. Without a token, GitHub API access is capped at 60 req/hour.

## Git Push & Release

When `git push` fails with HTTP2 framing errors (common on some networks), retry directly — it often succeeds on the second or third attempt. If it keeps failing, use `gh` to check auth (`gh auth status`) and retry.

Release workflow: the CI pipeline (`release.yml`) triggers on `v*` tag pushes. To cut a release:

```bash
# Bump version in Cargo.toml, then:
git add Cargo.toml Cargo.lock [other files]
git commit -m "feat: <summary>"
git push origin main
git tag -a v0.X.Y -m "v0.X.Y: <summary>"
git push origin v0.X.Y
```

CI builds for all 4 platforms (linux-x86_64, macos-aarch64, macos-x86_64, windows-x86_64) and publishes to the GitHub Releases page. Check status with `gh run list --workflow=release.yml --limit 3`.

## Data Storage

Database and cache live under the platform config dir under `musictui2/`:
- macOS: `~/Library/Application Support/musictui2/`
- Linux: `~/.config/musictui2/`
- Windows: `%APPDATA%\musictui2\`

The SQLite file is `music.db`; cached audio is `cache/<sha256>`. Schema migrations run on every startup in `DatabaseManager::from_path` — when adding columns, append to the migration block there rather than altering the `CREATE TABLE IF NOT EXISTS` statement (existing users have the old schema).

## Architecture

The app is a CLI-first Rust application; the TUI is one of the subcommands. Tokio drives async I/O end-to-end.

**Module boundaries** (`src/`):
- `main.rs` — clap subcommand dispatch; constructs `EventBus` and `Cli`.
- `cli/` — orchestrates `DatabaseManager`, `CacheManager`, `GitHubScanner`, `WebDavScanner`. All command handlers live here; `main.rs` only prints results.
- `models.rs` — `Repository`, `Track`, `RepositorySource` (`GitHub` | `WebDav`), `PlaybackState`. These types cross every module boundary.
- `database/` — single-file `DatabaseManager` wrapping `rusqlite::Connection`. **Schema migrations run inline in `from_path`**. Repositories carry a `source_type` discriminating GitHub vs WebDAV; WebDAV repos also store `username`/`password`/`cache_enabled`.
- `github/` — GitHub API client + recursive scanner; writes discovered tracks straight to the database.
- `webdav.rs` — WebDAV equivalent; honors `cache_enabled` per source (uncached sources stream from origin and skip writing the cache file).
- `cache/` — SHA256-keyed file cache; supports concurrent "caching while playing" (a partial file may exist while a download is in-flight).
- `audio/` — `rodio`-based playback; can play from a fully-cached file or stream while the cache writer runs.
- `events/` — `EventBus` built on `std::sync::mpsc`. Currently mostly unused (`_event_bus` in `Cli`), but the event enum is the contract for future cross-module signaling.
- `tui/` — single large `App` struct in `tui/mod.rs::App` (four tabs: Repositories / Tracks / Favorites / Blacklist). Owns its own copies of the managers; `tui::run(event_bus)` is the entrypoint.

**Source abstraction**: `RepositorySource` is the dispatch point. When adding a new source backend, you need: (1) a new variant on the enum + `as_str`/`FromStr`, (2) a scanner with `add_source` / `scan_repository` / `download_track`, (3) dispatch arms in `cli/mod.rs::scan_repository` and `download_track`. The database schema already carries `username`/`password`/`cache_enabled` for source-specific config.

**Cache semantics**: A track is "playable" if `downloaded && local_path` exists. The TUI's `Cache` column distinguishes `Cached` (file complete), `Caching` (write in progress while playing), `-` (absent). Do not assume a cache file is complete just because it exists on disk.

## Notes for changes

- The crate has two parallel manifests of guidance (`README.md`, `AGENTS.md`). When user-facing behavior or commands change, both should be updated.
- TUI logic is centralized in one large `App` impl — when extending, prefer adding methods on `App` over introducing new top-level helpers, since state ownership flows from `App`.
- `unsafe impl Sync/Send for DatabaseManager` in `database/mod.rs` exists because the underlying `Connection` is wrapped behind a mutex internally; preserve this when refactoring the database layer.
