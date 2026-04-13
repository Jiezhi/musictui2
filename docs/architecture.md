# Musictui2 Architecture Diagram

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        Musictui2 System                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐        │
│  │   CLI       │    │   TUI       │    │   Audio     │        │
│  │  Interface  │◄──►│  Interface  │◄──►│  Player     │        │
│  │             │    │             │    │ (Rodio)     │        │
│  └─────────────┘    └─────────────┘    └─────────────┘        │
│         │                 │                  │                 │
│         ▼                 ▼                  ▼                 │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐        │
│  │   Config    │    │   Event     │    │   Cache     │        │
│  │  Manager    │    │  Bus        │    │  Manager    │        │
│  └─────────────┘    └─────────────┘    └─────────────┘        │
│         │                 │                  │                 │
│         └─────────┬───────┼─────────────────┼─────────────────┘
│                   │       │                 │                 │
│         ┌─────────▼───────▼─────────────────▼─────────────────┐│
│         │                    │                 │               ││
│         │        ┌─────────────────────────────────┐        ││
│         │        │          Core Services          │        ││
│         │        │                                 │        ││
│         │  ┌─────▼─────┐  ┌───────▼───────┐  ┌────▼────┐   ││
│         │  │GitHub     │  │Database      │  │File      │   ││
│         │  │Scanner    │  │Manager       │  │Manager   │   ││
│         │  └───────────┘  └──────────────┘  └──────────┘   ││
│         │        │                 │               │        ││
│         │        └─────────────────┼───────────────┘        ││
│         │                         │                        ││
│         └─────────────────────────┼─────────────────────────┘│
│                                  │                         ││
│                ┌──────────────────▼───────────────────┐       ││
│                │         External Services           │       ││
│                │                                  │       ││
│                │  ┌─────────────┐  ┌─────────────┐  │       ││
│                │  │GitHub API  │  │Local File   │  │       ││
│                │  │ (reqwest)   │  │ System      │  │       ││
│                │  └─────────────┘  └─────────────┘  │       ││
│                └────────────────────────────────────┘       ││
│                                                                 │
├─────────────────────────────────────────────────────────────────┤
│                    Platform Layer                               │
│                                                                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐               │
│  │   macOS     │  │   Windows   │  │   Linux     │               │
│  │ (Core Audio)│  │ (WASAPI)    │  │ (ALSA/Pulse)│               │
│  └─────────────┘  └─────────────┘  └─────────────┘               │
└─────────────────────────────────────────────────────────────────┘
```

## Module Responsibilities

### 1. CLI Interface (`src/cli/`)
**Purpose**: Command-line entry point and configuration management
**Key Interfaces**:
```rust
pub trait CliCommand {
    fn execute(&self) -> Result<(), Error>;
}

pub struct Cli {
    config: Arc<ConfigManager>,
    commands: Vec<Box<dyn CliCommand>>,
}
```

**Responsibilities**:
- Parse command-line arguments (using clap)
- Handle repository management (add/list/remove)
- Configuration management
- Launch TUI interface

### 2. TUI Interface (`src/tui/`)
**Purpose**: Terminal User Interface for music browsing and playback
**Key Interfaces**:
```rust
pub trait TuiComponent {
    fn render(&mut self, f: &mut Frame) -> Result<(), Error>;
    fn handle_event(&mut self, event: Event) -> Result<EventResult, Error>;
}

pub struct App {
    screen: Screen,
    components: Vec<Box<dyn TuiComponent>>,
    event_bus: EventBus,
}
```

**Responsibilities**:
- Render repository browser
- Display track lists
- Handle user input for navigation
- Show playback controls
- Display current track information
- Handle keyboard shortcuts

### 3. Audio Player (`src/audio/`)
**Purpose**: Cross-platform audio playback using Rodio
**Key Interfaces**:
```rust
pub trait AudioBackend {
    fn play(&mut self, source: AudioSource) -> Result<(), Error>;
    fn pause(&mut self) -> Result<(), Error>;
    fn stop(&mut self) -> Result<(), Error>;
    fn set_volume(&mut self, volume: f32) -> Result<(), Error>;
}

pub struct AudioPlayer {
    backend: Box<dyn AudioBackend>,
    current_track: Option<Track>,
    state: PlaybackState,
}
```

**Responsibilities**:
- Stream audio from local cache
- Handle playback controls (play/pause/stop/volume)
- Manage playback state
- Cross-platform audio output

### 4. GitHub Scanner (`src/github/`)
**Purpose**: Scan GitHub repositories for audio files
**Key Interfaces**:
```rust
pub trait GitHubClient {
    fn get_repositories(&self, owner: &str) -> Result<Vec<Repository>, Error>;
    fn scan_repository(&self, repo: &Repository) -> Result<Vec<AudioFile>, Error>;
    fn get_file_content(&self, url: &str) -> Result<Bytes, Error>;
}

pub struct GitHubScanner {
    client: Box<dyn GitHubClient>,
    cache: Arc<CacheManager>,
}
```

**Responsibilities**:
- Recursively scan repositories for audio files
- Filter supported formats (mp3, flac, wav, ogg, m4a)
- Fetch file metadata
- Handle rate limiting and authentication
- Cache API responses

### 5. Database Manager (`src/database/`)
**Purpose**: SQLite database for storing repository metadata
**Key Interfaces**:
```rust
pub trait Database {
    fn save_repository(&self, repo: &Repository) -> Result<(), Error>;
    fn get_repositories(&self) -> Result<Vec<Repository>, Error>;
    fn save_track(&self, track: &Track) -> Result<(), Error>;
    fn get_tracks_by_repo(&self, repo_id: i64) -> Result<Vec<Track>, Error>;
}

pub struct DatabaseManager {
    connection: Connection,
}
```

**Responsibilities**:
- Store repository information
- Cache track metadata
- User preferences
- Playback history
- Search indexing

### 6. Cache Manager (`src/cache/`)
**Purpose**: Manage local file caching for audio files
**Key Interfaces**:
```rust
pub trait Cache {
    fn get(&self, key: &str) -> Result<Option<PathBuf>, Error>;
    fn put(&self, key: &str, data: &[u8]) -> Result<PathBuf, Error>;
    fn exists(&self, key: &str) -> bool;
    fn cleanup(&self) -> Result<(), Error>;
}

pub struct CacheManager {
    cache_dir: PathBuf,
    cache: Box<dyn Cache>,
}
```

**Responsibilities**:
- Download and cache audio files
- Manage cache size and eviction
- Cache validation
- File deduplication

## Data Flow

```
User Input → CLI/TUI → Event Bus → Core Services → External APIs → Cache → Audio Player → Output
     ↑                                                                      ↓
  Configuration ← Database Manager ← GitHub Scanner ← GitHub API
```

### Typical Playback Flow:
1. User adds GitHub repository via CLI
2. GitHub Scanner fetches repository structure
3. Database stores repository metadata
4. User browses repositories in TUI
5. GitHub Scanner downloads audio files to cache
6. Audio Player streams from cache
7. TUI updates playback state

## Cross-Platform Considerations

### Audio Backends
- **macOS**: Core Audio (via rodio's cpal backend)
- **Windows**: WASAPI (via rodio's cpal backend)
- **Linux**: ALSA/PulseAudio (via rodio's cpal backend)

### File System
- Use `std::path` for cross-platform path handling
- Implement proper directory structure for cache
- Handle file permissions appropriately

### Dependencies
- **ratatui**: Cross-platform TUI framework
- **rodio**: Cross-platform audio playback
- **reqwest**: HTTP client with native TLS
- **rusqlite**: SQLite bindings

## Security Guidelines

### GitHub API Access
- Use anonymous API access for public repositories
- Support GitHub authentication for private repositories
- Implement rate limiting respect
- Never store GitHub tokens in plaintext

### File System Security
- Validate all downloaded files
- Scan for potential malicious content
- Restrict cache directory access
- Implement proper file permissions

### Input Validation
- Sanitize all user inputs
- Validate repository URLs
- Check file extensions before download
- Implement size limits for downloads

## Testing Strategy

### Unit Tests
- Audio player mock and testing
- Database operations with in-memory SQLite
- GitHub API mocking
- Cache operations with test files

### Integration Tests
- End-to-end repository scanning
- TUI event handling
- Audio playback with test files
- Database persistence

### E2E Tests
- Full workflow from CLI to playback
- Error handling scenarios
- Performance under load
- Cross-platform compatibility

### Test Coverage Requirements
- Minimum 80% test coverage
- All public API functions tested
- Error paths covered
- Edge cases handled

## Key Design Patterns

### Repository Pattern
- Database operations abstracted behind traits
- Easy swapping of storage backends
- Clean separation of concerns

### Event-Driven Architecture
- Central event bus for communication
- Loose coupling between components
- Reactive UI updates

### Strategy Pattern
- Pluggable audio backends
- Configurable cache strategies
- Extensible GitHub clients

### Factory Pattern
- Create appropriate platform-specific implementations
- Centralized object creation
- Consistent initialization

## Performance Considerations

### Caching Strategy
- LRU cache for frequently accessed files
- Preload next track for seamless playback
- Background downloading of popular tracks

### Memory Management
- Stream audio files rather than loading entirely
- Implement proper cleanup of unused resources
- Monitor memory usage and implement limits

### Concurrency
- Async I/O for network operations
- Background scanning and downloading
- Thread-safe audio playback

## Future Extensibility

### Plugin System
- Support for additional audio sources
- Custom TUI themes
- Extended metadata formats

### Advanced Features
- Playlist management
- Streaming quality options
- Lyrics integration
- Audio visualizations
