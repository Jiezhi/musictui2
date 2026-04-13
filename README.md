# Musictui2

A cross-platform terminal-based music player that streams audio files directly from GitHub repositories.

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.71+-orange.svg)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)

## Features

- 🎵 **GitHub Integration**: Scan GitHub repositories for audio files (mp3, flac, wav, ogg, etc.)
- 🖥️ **Terminal UI**: Built with [ratatui](https://github.com/ratatui-org/ratatui) for a rich terminal experience
- 🎶 **Audio Playback**: Cross-platform audio output using [rodio](https://github.com/RustAudio/rodio)
- 💾 **Local Caching**: Automatic caching of downloaded audio files
- 🗄️ **SQLite Storage**: Metadata stored locally in SQLite database
- 🎛️ **Playback Controls**: Play, pause, stop, and volume control
- 🔄 **Recursive Scanning**: Deep scan repositories for audio files in subdirectories

## Quick Start

### Prerequisites

- Rust 1.71 or higher
- GitHub API token (for higher rate limits)

### Installation

1. Clone the repository:
```bash
git clone https://github.com/yourusername/musictui2.git
cd musictui2
```

2. Build the project:
```bash
cargo build --release
```

3. Run the application:
```bash
cargo run --release
```

### Usage

#### Adding a Repository

```bash
musictui2 add-repo <owner> <repo>
# Example: musictui2 add-repo torvalds linux
```

#### Listing Repositories

```bash
musictui2 list-repos
```

#### Removing a Repository

```bash
musictui2 remove-repo <repo-id>
```

#### Running the TUI

```bash
musictui2
```

The TUI provides:
- Repository browser
- Track listing with metadata
- Playback controls
- Volume adjustment
- Progress indicator

## Configuration

The application stores configuration and cache in:
- **macOS**: `~/Library/Application Support/musictui2/`
- **Linux**: `~/.config/musictui2/`
- **Windows**: `%APPDATA%\musictui2\`

## GitHub API Rate Limits

Without a GitHub API token, you're limited to 60 requests per hour. To increase this:

1. Create a GitHub Personal Access Token
2. Set the `GITHUB_API_TOKEN` environment variable:
```bash
export GITHUB_API_TOKEN=your_token_here
```

## Development

### Running Tests

```bash
cargo test
```

### Code Formatting

```bash
cargo fmt
```

### Linting

```bash
cargo clippy
```

## Project Structure

```
src/
├── main.rs              # Application entry point
├── lib.rs               # Library exports
├── models.rs            # Data models (Track, Repository, etc.)
├── audio/               # Audio playback module
│   └── mod.rs
├── database/            # SQLite database operations
│   └── mod.rs
├── github/              # GitHub API integration
│   └── mod.rs
├── tui/                 # Terminal User Interface
│   └── mod.rs
├── cli/                 # Command-line interface
│   └── mod.rs
├── cache/               # File caching system
│   └── mod.rs
└── events/              # Event system
    └── mod.rs
```

## Contributing

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- [ratatui](https://github.com/ratatui-org/ratatui) for the terminal UI framework
- [rodio](https://github.com/RustAudio/rodio) for audio playback
- [rusqlite](https://github.com/rusqlite/rusqlite) for SQLite integration
- [clap](https://github.com/clap-rs/clap) for CLI parsing
