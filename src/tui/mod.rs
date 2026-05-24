mod draw;
mod util;

use crate::audio::{prepare_streaming_decoder, AudioPlayer, StreamingDecoder};
use crate::cache::{CacheManager, StreamingCacheState};
use crate::credentials::{resolve_webdav_password, CredentialStore, KeyringStore};
use crate::database::DatabaseManager;
use crate::events::EventBus;
use crate::github::GitHubScanner;
use crate::models::{Repository, RepositorySource, Track};
use crate::webdav::WebDavScanner;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute, terminal,
};
use ratatui::{
    backend::CrosstermBackend,
    widgets::{ListState, TableState},
    Terminal,
};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinHandle;

use draw::{render_frame, DrawData};
use util::{
    filtered_track_indices, initial_buffer_bytes, next_repository_index, next_track_index,
    previous_repository_index, previous_track_index, sequential_autoplay_index,
    should_handle_key_event, shuffle_autoplay_index, track_list_filter_for_tab, volume_percent,
    TrackListFilter, DEFAULT_TRACKS_PAGE_STEP, TAB_BLACKLIST, TAB_COUNT, TAB_REPOSITORIES,
};

struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaybackMode {
    Sequential,
    Shuffle,
}

impl PlaybackMode {
    fn next(self) -> Self {
        match self {
            Self::Sequential => Self::Shuffle,
            Self::Shuffle => Self::Sequential,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Sequential => "Sequential",
            Self::Shuffle => "Shuffle",
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Shuffle => "shuffle",
        }
    }
}

impl std::str::FromStr for PlaybackMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "sequential" => Ok(Self::Sequential),
            "shuffle" => Ok(Self::Shuffle),
            other => Err(format!("unknown playback mode: {other}")),
        }
    }
}

const PLAYBACK_MODE_SETTING_KEY: &str = "playback_mode";

struct PendingPlayback {
    track_index: usize,
    track: Track,
    state: PendingPlaybackState,
}

enum PendingPlaybackState {
    Preparing {
        cache_state: StreamingCacheState,
        download_handle: JoinHandle<Result<PathBuf, String>>,
        decoder_handle: JoinHandle<Result<StreamingDecoder, String>>,
    },
    Caching {
        cache_state: StreamingCacheState,
        download_handle: JoinHandle<Result<PathBuf, String>>,
    },
}

impl PendingPlayback {
    fn cancel(self) {
        match self.state {
            PendingPlaybackState::Preparing {
                cache_state,
                download_handle,
                decoder_handle,
            } => {
                cache_state.mark_error("Playback cancelled");
                download_handle.abort();
                decoder_handle.abort();
            }
            PendingPlaybackState::Caching {
                cache_state,
                download_handle,
            } => {
                cache_state.mark_error("Playback cancelled");
                download_handle.abort();
            }
        }
    }

    fn cache_progress(&self) -> Option<(u64, u64)> {
        let cache_state = match &self.state {
            PendingPlaybackState::Preparing { cache_state, .. }
            | PendingPlaybackState::Caching { cache_state, .. } => cache_state,
        };

        Some((cache_state.downloaded_bytes(), self.track.size))
    }

    fn cache_percent(&self) -> Option<u64> {
        let (downloaded_bytes, total_bytes) = self.cache_progress()?;
        if total_bytes == 0 {
            return None;
        }

        Some((downloaded_bytes.saturating_mul(100) / total_bytes).min(100))
    }
}

fn pending_status(action: &str, pending: &PendingPlayback, track_name: &str) -> String {
    if let Some(percent) = pending.cache_percent() {
        format!("{action} {track_name} | Caching {percent}%")
    } else {
        let downloaded_mb = pending
            .cache_progress()
            .map(|(downloaded_bytes, _)| downloaded_bytes / 1024 / 1024)
            .unwrap_or_default();
        format!("{action} {track_name} | Caching {downloaded_mb}MB")
    }
}

pub struct App {
    terminal: Option<Terminal<CrosstermBackend<io::Stdout>>>,
    selected_tab: usize,
    repositories: Vec<Repository>,
    tracks: Vec<Track>,
    visible_track_indices: Vec<usize>,
    repositories_list_state: ListState,
    tracks_table_state: TableState,
    tracks_page_step: usize,
    audio_player: AudioPlayer,
    current_repository_index: Option<usize>,
    current_track_index: Option<usize>,
    current_track_row_index: Option<usize>,
    playback_mode: PlaybackMode,
    shuffle_seed: u64,
    pending_repository_delete: Option<usize>,
    #[allow(dead_code)]
    event_bus: EventBus,
    database: Arc<DatabaseManager>,
    #[allow(dead_code)]
    github_scanner: Arc<GitHubScanner>,
    cache: Arc<CacheManager>,
    credential_store: Arc<dyn CredentialStore>,
    pending_playback: Option<PendingPlayback>,
    status_message: String,
    track_search_query: String,
    is_searching_tracks: bool,
    show_help: bool,
    should_quit: bool,
}

impl App {
    pub fn new(
        event_bus: EventBus,
        database: Arc<DatabaseManager>,
        github_scanner: Arc<GitHubScanner>,
        cache: Arc<CacheManager>,
        credential_store: Arc<dyn CredentialStore>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let terminal = None;
        let audio_player = AudioPlayer::with_github_scanner(github_scanner.clone())?;
        let shuffle_seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0x05ee_d5ee_dd15_ca11_u64);

        let playback_mode = database
            .get_setting(PLAYBACK_MODE_SETTING_KEY)
            .ok()
            .flatten()
            .and_then(|value| value.parse::<PlaybackMode>().ok())
            .unwrap_or(PlaybackMode::Sequential);

        Ok(Self {
            terminal,
            selected_tab: 0,
            repositories: Vec::new(),
            tracks: Vec::new(),
            visible_track_indices: Vec::new(),
            repositories_list_state: ListState::default(),
            tracks_table_state: TableState::default(),
            tracks_page_step: DEFAULT_TRACKS_PAGE_STEP,
            audio_player,
            current_repository_index: None,
            current_track_index: None,
            current_track_row_index: None,
            playback_mode,
            shuffle_seed,
            pending_repository_delete: None,
            event_bus,
            database,
            github_scanner,
            cache,
            credential_store,
            pending_playback: None,
            status_message: "Ready".to_string(),
            track_search_query: String::new(),
            is_searching_tracks: false,
            show_help: false,
            should_quit: false,
        })
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut stdout = io::stdout();
        terminal::enable_raw_mode()?;
        let _cleanup = TerminalCleanup;
        execute!(
            stdout,
            terminal::EnterAlternateScreen,
            crossterm::event::EnableMouseCapture
        )?;

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        self.terminal = Some(terminal);

        self.load_data()?;

        let mut last_tick = std::time::Instant::now();
        let tick_rate = std::time::Duration::from_millis(250);

        while !self.should_quit {
            if let Ok(true) = event::poll(tick_rate) {
                if let Event::Key(key) = event::read()? {
                    if should_handle_key_event(key.kind) {
                        if let Err(err) = self.handle_key_event(key).await {
                            self.show_error(&err.to_string());
                        }
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                self.on_tick().await?;
                last_tick = std::time::Instant::now();
            }

            let draw_data = DrawData {
                repositories: &self.repositories,
                tracks: &self.tracks,
                visible_track_indices: &self.visible_track_indices,
                track_list_filter: self.current_track_list_filter(),
                selected_tab: self.selected_tab,
                current_repository_index: self.current_repository_index,
                current_track_index: self.current_track_index,
                current_track_row_index: self.current_track_row_index,
                caching_track_index: self
                    .pending_playback
                    .as_ref()
                    .map(|pending| pending.track_index),
                playback_state: self.audio_player.get_playback_state(),
                playback_mode: self.playback_mode,
                volume: self.audio_player.get_volume(),
                status_message: &self.status_message,
                track_search_query: &self.track_search_query,
                is_searching_tracks: self.is_searching_tracks,
                show_help: self.show_help,
            };

            let repositories_list_state = &mut self.repositories_list_state;
            let tracks_table_state = &mut self.tracks_table_state;
            let tracks_page_step = &mut self.tracks_page_step;

            if let Some(terminal) = &mut self.terminal {
                terminal.draw(|f| {
                    render_frame(
                        f,
                        &draw_data,
                        repositories_list_state,
                        tracks_table_state,
                        tracks_page_step,
                    );
                })?;
            }
        }

        self.terminal = None;

        Ok(())
    }

    fn load_data(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.repositories = self.database.get_repositories()?;
        self.tracks = self.database.get_all_tracks()?;
        self.refresh_visible_tracks();
        self.select_repository_index(self.current_repository_index);
        self.select_track_index(self.current_track_index);
        Ok(())
    }

    fn refresh_visible_tracks(&mut self) {
        let filter = self.current_track_list_filter();
        self.visible_track_indices =
            filtered_track_indices(&self.tracks, &self.track_search_query, filter);
        self.sync_selected_track_with_visible_tracks();
    }

    fn current_track_list_filter(&self) -> TrackListFilter {
        track_list_filter_for_tab(self.selected_tab).unwrap_or(TrackListFilter::Tracks)
    }

    fn selected_tab_is_track_list(&self) -> bool {
        track_list_filter_for_tab(self.selected_tab).is_some()
    }

    fn selected_tab_can_play_tracks(&self) -> bool {
        track_list_filter_for_tab(self.selected_tab).is_some_and(TrackListFilter::can_play)
    }

    fn select_tab(&mut self, tab: usize) {
        self.selected_tab = tab % TAB_COUNT;
        self.refresh_visible_tracks();
    }

    fn sync_selected_track_with_visible_tracks(&mut self) {
        if self.visible_track_indices.is_empty() {
            self.current_track_index = None;
            self.current_track_row_index = None;
            self.tracks_table_state.select(None);
            return;
        }

        let row_index = self
            .current_track_index
            .and_then(|track_index| {
                self.visible_track_indices
                    .iter()
                    .position(|&visible_index| visible_index == track_index)
            })
            .unwrap_or(0);
        let track_index = self.visible_track_indices[row_index];

        self.current_track_index = Some(track_index);
        self.current_track_row_index = Some(row_index);
        self.tracks_table_state.select(Some(row_index));
    }

    fn search_status_message(&self) -> String {
        if self.track_search_query.is_empty() {
            format!(
                "Search {}",
                self.current_track_list_filter().label().to_lowercase()
            )
        } else {
            format!(
                "Search {}: {} ({} matches)",
                self.current_track_list_filter().label().to_lowercase(),
                self.track_search_query,
                self.visible_track_indices.len()
            )
        }
    }

    async fn handle_key_event(&mut self, key: KeyEvent) -> Result<(), Box<dyn std::error::Error>> {
        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    self.show_help = false;
                }
                KeyCode::Char('q') => {
                    self.should_quit = true;
                }
                _ => {}
            }

            return Ok(());
        }

        if self.is_searching_tracks {
            return self.handle_track_search_key_event(key);
        }

        match key.code {
            KeyCode::Esc => {
                if !self.track_search_query.is_empty() {
                    self.track_search_query.clear();
                    self.refresh_visible_tracks();
                    self.status_message = "Search cleared".to_string();
                } else {
                    self.status_message = "Ready".to_string();
                }
                self.pending_repository_delete = None;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                self.pending_repository_delete = None;
            }
            KeyCode::Char('/') if self.selected_tab_is_track_list() => {
                self.is_searching_tracks = true;
                self.pending_repository_delete = None;
                self.status_message = format!(
                    "Search {}",
                    self.current_track_list_filter().label().to_lowercase()
                );
            }
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Char('n') => {
                self.select_tab((self.selected_tab + 1) % TAB_COUNT);
                self.pending_repository_delete = None;
            }
            KeyCode::Char('p') => {
                self.select_tab((self.selected_tab + TAB_COUNT - 1) % TAB_COUNT);
                self.pending_repository_delete = None;
            }
            KeyCode::Tab => {
                self.select_tab((self.selected_tab + 1) % TAB_COUNT);
                self.pending_repository_delete = None;
            }
            KeyCode::BackTab => {
                self.select_tab((self.selected_tab + TAB_COUNT - 1) % TAB_COUNT);
                self.pending_repository_delete = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_tab == TAB_REPOSITORIES {
                    self.select_previous_repository();
                } else if self.selected_tab_is_track_list() {
                    self.select_previous_track(1);
                }
                self.pending_repository_delete = None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected_tab == TAB_REPOSITORIES {
                    self.select_next_repository();
                } else if self.selected_tab_is_track_list() {
                    self.select_next_track(1);
                }
                self.pending_repository_delete = None;
            }
            KeyCode::PageUp if self.selected_tab_is_track_list() => {
                self.select_previous_track(self.tracks_page_step);
                self.pending_repository_delete = None;
            }
            KeyCode::PageDown if self.selected_tab_is_track_list() => {
                self.select_next_track(self.tracks_page_step);
                self.pending_repository_delete = None;
            }
            KeyCode::Char('b')
                if self.selected_tab_is_track_list()
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.select_previous_track(self.tracks_page_step);
                self.pending_repository_delete = None;
            }
            KeyCode::Char('f')
                if self.selected_tab_is_track_list()
                    && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.select_next_track(self.tracks_page_step);
                self.pending_repository_delete = None;
            }
            KeyCode::Char('m') if self.selected_tab_is_track_list() => {
                self.playback_mode = self.playback_mode.next();
                if let Err(err) = self
                    .database
                    .set_setting(PLAYBACK_MODE_SETTING_KEY, self.playback_mode.as_str())
                {
                    self.status_message = format!(
                        "Playback mode: {} (save failed: {err})",
                        self.playback_mode.label()
                    );
                } else {
                    self.status_message = format!("Playback mode: {}", self.playback_mode.label());
                }
                self.pending_repository_delete = None;
            }
            KeyCode::Char('.')
                if self.selected_tab_can_play_tracks()
                    && !self.visible_track_indices.is_empty() =>
            {
                self.play_next_track()?;
                self.pending_repository_delete = None;
            }
            KeyCode::Char(',')
                if self.selected_tab_can_play_tracks()
                    && !self.visible_track_indices.is_empty() =>
            {
                self.play_previous_track()?;
                self.pending_repository_delete = None;
            }
            KeyCode::Enter
                if self.selected_tab_can_play_tracks()
                    && !self.visible_track_indices.is_empty() =>
            {
                if let Some(index) = self.current_track_index {
                    self.queue_track(index)?;
                }
                self.pending_repository_delete = None;
            }
            KeyCode::Char('f') if self.selected_tab_is_track_list() => {
                self.toggle_selected_track_favorite()?;
                self.pending_repository_delete = None;
            }
            KeyCode::Char('x') if self.selected_tab_is_track_list() => {
                self.toggle_selected_track_blacklisted()?;
                self.pending_repository_delete = None;
            }
            KeyCode::Char('d') if self.selected_tab == TAB_REPOSITORIES => {
                self.confirm_or_delete_selected_repository()?;
            }
            KeyCode::Char(' ') => {
                if self.audio_player.is_playing() {
                    self.audio_player.pause()?;
                    self.status_message = "Paused".to_string();
                } else if self.audio_player.is_paused() {
                    self.audio_player.play()?;
                    self.status_message = "Playing".to_string();
                } else if self.selected_tab_can_play_tracks() {
                    if let Some(index) = self.current_track_index {
                        self.queue_track(index)?;
                    }
                } else if self.selected_tab == TAB_BLACKLIST {
                    self.status_message = "Restore a blacklisted track before playing".to_string();
                } else if let Some(index) = self.playing_track_index() {
                    self.queue_track(index)?;
                }
                self.pending_repository_delete = None;
            }
            KeyCode::Char('+') => {
                let new_volume = (self.audio_player.get_volume() + 0.1).min(1.0);
                self.audio_player.set_volume(new_volume)?;
                self.status_message = format!("Volume: {}%", volume_percent(new_volume));
                self.pending_repository_delete = None;
            }
            KeyCode::Char('-') => {
                let new_volume = (self.audio_player.get_volume() - 0.1).max(0.0);
                self.audio_player.set_volume(new_volume)?;
                self.status_message = format!("Volume: {}%", volume_percent(new_volume));
                self.pending_repository_delete = None;
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_track_search_key_event(
        &mut self,
        key: KeyEvent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match key.code {
            KeyCode::Esc => {
                self.is_searching_tracks = false;
                if self.track_search_query.is_empty() {
                    self.status_message = "Ready".to_string();
                } else {
                    self.track_search_query.clear();
                    self.refresh_visible_tracks();
                    self.status_message = "Search cleared".to_string();
                }
                self.pending_repository_delete = None;
            }
            KeyCode::Enter => {
                self.is_searching_tracks = false;
                self.status_message = if self.track_search_query.is_empty() {
                    "Ready".to_string()
                } else {
                    format!(
                        "Search {}: {} ({} matches)",
                        self.current_track_list_filter().label().to_lowercase(),
                        self.track_search_query,
                        self.visible_track_indices.len()
                    )
                };
                self.pending_repository_delete = None;
            }
            KeyCode::Backspace => {
                self.track_search_query.pop();
                self.refresh_visible_tracks();
                self.status_message = self.search_status_message();
                self.pending_repository_delete = None;
            }
            KeyCode::Char(character)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.track_search_query.push(character);
                self.refresh_visible_tracks();
                self.status_message = self.search_status_message();
                self.pending_repository_delete = None;
            }
            _ => {}
        }

        Ok(())
    }

    fn queue_track(&mut self, index: usize) -> Result<(), Box<dyn std::error::Error>> {
        if index < self.tracks.len() {
            self.select_track_index(Some(index));
            let track = self.tracks[index].clone();

            if track.blacklisted {
                self.status_message = format!("Restore {} before playing", track.name);
                return Ok(());
            }

            if track.is_playable() {
                if let Some(pending) = self.pending_playback.take() {
                    pending.cancel();
                }
                self.audio_player.load_local_track(track.clone())?;
                self.status_message = format!("Playing {}", track.name);
                return Ok(());
            }

            if let Some(pending) = self.pending_playback.take() {
                pending.cancel();
            }

            let repository = self.database.get_repository_by_id(track.repository_id)?;
            let streaming_download = match repository.source_type {
                RepositorySource::GitHub => self
                    .github_scanner
                    .start_streaming_download(track.clone())?,
                RepositorySource::WebDav => {
                    let password = resolve_webdav_password(
                        &repository.name,
                        repository.password.as_deref(),
                        self.credential_store.as_ref(),
                    )
                    .unwrap_or_else(|_| repository.password.clone());
                    let scanner = WebDavScanner::new(
                        self.database.clone(),
                        self.cache.clone(),
                        repository.username,
                        password,
                    );
                    scanner.start_streaming_download(track.clone(), repository.cache_enabled)?
                }
            };
            let decoder_cache_path = streaming_download.cache_path.clone();
            let decoder_cache_state = streaming_download.state.clone();
            let format = track.format.clone();
            let buffer_bytes = initial_buffer_bytes(&track);
            let decoder_handle = tokio::task::spawn_blocking(move || {
                prepare_streaming_decoder(
                    decoder_cache_path,
                    decoder_cache_state,
                    format,
                    buffer_bytes,
                )
            });

            self.pending_playback = Some(PendingPlayback {
                track_index: index,
                track: track.clone(),
                state: PendingPlaybackState::Preparing {
                    cache_state: streaming_download.state,
                    download_handle: streaming_download.handle,
                    decoder_handle,
                },
            });
            self.status_message = format!("Buffering {}", track.name);
        }
        Ok(())
    }

    async fn on_tick(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(pending) = self.pending_playback.take() {
            self.handle_pending_playback(pending).await?;
        }

        if self.audio_player.has_finished() {
            self.handle_track_finished()?;
        } else if self.audio_player.is_playing() {
            if let Some(track) = self.audio_player.get_current_track() {
                self.status_message = format!("Playing {}", track.name);
            }
        }

        Ok(())
    }

    async fn handle_pending_playback(
        &mut self,
        pending: PendingPlayback,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match pending.state {
            PendingPlaybackState::Preparing {
                cache_state,
                download_handle,
                decoder_handle,
            } => {
                if decoder_handle.is_finished() {
                    match decoder_handle.await {
                        Ok(Ok(decoder)) => {
                            let track = pending.track.clone();
                            self.audio_player
                                .load_streaming_track(track.clone(), decoder)?;
                            self.select_track_index(Some(pending.track_index));
                            let pending = PendingPlayback {
                                track_index: pending.track_index,
                                track,
                                state: PendingPlaybackState::Caching {
                                    cache_state,
                                    download_handle,
                                },
                            };
                            self.status_message =
                                pending_status("Playing", &pending, pending.track.name.as_str());
                            self.pending_playback = Some(pending);
                        }
                        Ok(Err(err)) => {
                            cache_state.mark_error(err.clone());
                            download_handle.abort();
                            self.status_message = format!("Playback failed: {}", err);
                        }
                        Err(err) => {
                            cache_state.mark_error(err.to_string());
                            download_handle.abort();
                            self.status_message = format!("Playback task failed: {}", err);
                        }
                    }
                } else {
                    let pending = PendingPlayback {
                        track_index: pending.track_index,
                        track: pending.track,
                        state: PendingPlaybackState::Preparing {
                            cache_state,
                            download_handle,
                            decoder_handle,
                        },
                    };
                    self.status_message =
                        pending_status("Buffering", &pending, pending.track.name.as_str());
                    self.pending_playback = Some(pending);
                }
            }
            PendingPlaybackState::Caching {
                cache_state,
                download_handle,
            } => {
                if download_handle.is_finished() {
                    match download_handle.await {
                        Ok(Ok(local_path)) => {
                            let mut track = pending.track;
                            track.local_path = Some(local_path);
                            track.downloaded = true;

                            if let Some(stored_track) = self.tracks.get_mut(pending.track_index) {
                                *stored_track = track.clone();
                            }

                            if self.audio_player.is_playing() {
                                self.status_message = format!("Playing {} | Cached", track.name);
                            } else {
                                self.status_message = format!("Cached {}", track.name);
                            }
                        }
                        Ok(Err(err)) => {
                            cache_state.mark_error(err.clone());
                            self.status_message = format!("Cache failed: {}", err);
                        }
                        Err(err) => {
                            cache_state.mark_error(err.to_string());
                            self.status_message = format!("Cache task failed: {}", err);
                        }
                    }
                } else {
                    let pending = PendingPlayback {
                        track_index: pending.track_index,
                        track: pending.track,
                        state: PendingPlaybackState::Caching {
                            cache_state,
                            download_handle,
                        },
                    };
                    self.status_message =
                        pending_status("Playing", &pending, pending.track.name.as_str());
                    self.pending_playback = Some(pending);
                }
            }
        }

        Ok(())
    }

    fn select_track_index(&mut self, index: Option<usize>) {
        let Some(index) = index else {
            self.current_track_index = None;
            self.current_track_row_index = None;
            self.tracks_table_state.select(None);
            return;
        };

        if self.tracks.is_empty() {
            self.current_track_index = None;
            self.current_track_row_index = None;
            self.tracks_table_state.select(None);
            return;
        }

        let index = index.min(self.tracks.len() - 1);
        let row_index = self
            .visible_track_indices
            .iter()
            .position(|&visible_index| visible_index == index);

        self.current_track_index = row_index.map(|_| index);
        self.current_track_row_index = row_index;
        self.tracks_table_state.select(row_index);
    }

    fn select_repository_index(&mut self, index: Option<usize>) {
        let index = index
            .filter(|_| !self.repositories.is_empty())
            .map(|index| index.min(self.repositories.len() - 1));
        self.current_repository_index = index;
        self.repositories_list_state.select(index);
    }

    fn select_next_track(&mut self, step: usize) {
        let row_index = next_track_index(
            self.current_track_row_index,
            self.visible_track_indices.len(),
            step,
        );
        self.select_visible_track_row(row_index);
    }

    fn select_previous_track(&mut self, step: usize) {
        let row_index = previous_track_index(
            self.current_track_row_index,
            self.visible_track_indices.len(),
            step,
        );
        self.select_visible_track_row(row_index);
    }

    fn select_visible_track_row(&mut self, row_index: Option<usize>) {
        let row_index = row_index
            .filter(|_| !self.visible_track_indices.is_empty())
            .map(|row_index| row_index.min(self.visible_track_indices.len() - 1));
        self.current_track_row_index = row_index;
        self.current_track_index = row_index.map(|row| self.visible_track_indices[row]);
        self.tracks_table_state.select(row_index);
    }

    fn selected_track_mut(&mut self) -> Option<&mut Track> {
        let index = self.current_track_index?;
        self.tracks.get_mut(index)
    }

    fn toggle_selected_track_favorite(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(index) = self.current_track_index else {
            self.status_message = "No track selected".to_string();
            return Ok(());
        };

        let Some(track) = self.tracks.get(index).cloned() else {
            self.select_track_index(None);
            self.status_message = "No track selected".to_string();
            return Ok(());
        };

        if track.blacklisted {
            self.status_message = format!("Restore {} before favoriting", track.name);
            return Ok(());
        }

        let favorite = !track.favorite;
        self.database.set_track_favorite(track.id, favorite)?;

        if let Some(stored_track) = self.selected_track_mut() {
            stored_track.favorite = favorite;
        }
        self.refresh_visible_tracks();
        self.status_message = if favorite {
            format!("Favorited {}", track.name)
        } else {
            format!("Removed favorite {}", track.name)
        };

        Ok(())
    }

    fn toggle_selected_track_blacklisted(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(index) = self.current_track_index else {
            self.status_message = "No track selected".to_string();
            return Ok(());
        };

        let Some(track) = self.tracks.get(index).cloned() else {
            self.select_track_index(None);
            self.status_message = "No track selected".to_string();
            return Ok(());
        };

        let blacklisted = !track.blacklisted;
        self.database.set_track_blacklisted(track.id, blacklisted)?;

        if let Some(stored_track) = self.selected_track_mut() {
            stored_track.blacklisted = blacklisted;
            if blacklisted {
                stored_track.favorite = false;
            }
        }

        if blacklisted {
            if self
                .audio_player
                .get_current_track()
                .is_some_and(|current_track| current_track.id == track.id)
            {
                self.audio_player.stop()?;
            }

            if let Some(pending) = self.pending_playback.take() {
                if pending.track.id == track.id {
                    pending.cancel();
                } else {
                    self.pending_playback = Some(pending);
                }
            }
        }

        self.refresh_visible_tracks();
        self.status_message = if blacklisted {
            format!("Blacklisted {}", track.name)
        } else {
            format!("Restored {}", track.name)
        };

        Ok(())
    }

    fn select_next_repository(&mut self) {
        let index = next_repository_index(self.current_repository_index, self.repositories.len());
        self.select_repository_index(index);
    }

    fn select_previous_repository(&mut self) {
        let index =
            previous_repository_index(self.current_repository_index, self.repositories.len());
        self.select_repository_index(index);
    }

    fn play_next_track(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let current = self.playing_track_index().or(self.current_track_index);
        if let Some(index) = self.next_playback_index_from(current, true) {
            self.queue_track(index)?;
        }

        Ok(())
    }

    fn play_previous_track(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let current = self.playing_track_index().or(self.current_track_index);
        let playback_indices = self.active_playback_indices();
        let current_row = current.and_then(|track_index| {
            playback_indices
                .iter()
                .position(|&visible_index| visible_index == track_index)
        });
        let index = previous_track_index(current_row, playback_indices.len(), 1)
            .map(|row| playback_indices[row]);
        if let Some(index) = index {
            self.queue_track(index)?;
        }

        Ok(())
    }

    fn handle_track_finished(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let finished_track_name = self
            .audio_player
            .get_current_track()
            .map(|track| track.name.clone())
            .unwrap_or_else(|| "Track".to_string());
        let finished_index = self.playing_track_index();

        self.audio_player.stop()?;

        if let Some(index) = self.next_playback_index_from(finished_index, false) {
            self.queue_track(index)?;
        } else {
            self.status_message = format!("{finished_track_name} finished");
        }

        Ok(())
    }

    fn next_playback_index_from(
        &mut self,
        current: Option<usize>,
        wrap_sequential: bool,
    ) -> Option<usize> {
        let playback_indices = self.active_playback_indices();
        match self.playback_mode {
            PlaybackMode::Sequential => {
                let current_row = current.and_then(|track_index| {
                    playback_indices
                        .iter()
                        .position(|&visible_index| visible_index == track_index)
                });
                let next_row = sequential_autoplay_index(current_row, playback_indices.len());
                if let Some(row) = next_row {
                    Some(playback_indices[row])
                } else if wrap_sequential && !playback_indices.is_empty() {
                    Some(playback_indices[0])
                } else {
                    None
                }
            }
            PlaybackMode::Shuffle => {
                let current_row = current.and_then(|track_index| {
                    playback_indices
                        .iter()
                        .position(|&visible_index| visible_index == track_index)
                });
                shuffle_autoplay_index(current_row, playback_indices.len(), &mut self.shuffle_seed)
                    .map(|row| playback_indices[row])
            }
        }
    }

    fn active_playback_indices(&self) -> Vec<usize> {
        if self.selected_tab_can_play_tracks() {
            self.visible_track_indices.clone()
        } else {
            filtered_track_indices(
                &self.tracks,
                &self.track_search_query,
                TrackListFilter::Tracks,
            )
        }
    }

    fn playing_track_index(&self) -> Option<usize> {
        let current_track_id = self.audio_player.get_current_track()?.id;
        self.tracks
            .iter()
            .position(|track| track.id == current_track_id)
    }

    fn confirm_or_delete_selected_repository(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(index) = self.current_repository_index else {
            self.status_message = "No repository selected".to_string();
            return Ok(());
        };

        let Some(repository) = self.repositories.get(index).cloned() else {
            self.select_repository_index(None);
            self.status_message = "No repository selected".to_string();
            return Ok(());
        };

        if self.pending_repository_delete != Some(index) {
            self.pending_repository_delete = Some(index);
            self.status_message = format!(
                "Press d again to delete {}/{} and its tracks/cache",
                repository.owner, repository.name
            );
            return Ok(());
        }

        self.delete_repository(repository)?;
        self.pending_repository_delete = None;
        Ok(())
    }

    fn delete_repository(
        &mut self,
        repository: Repository,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let repository_id = repository.id;

        if let Some(pending) = self.pending_playback.take() {
            if pending.track.repository_id == repository_id {
                pending.cancel();
            } else {
                self.pending_playback = Some(pending);
            }
        }

        if self
            .audio_player
            .get_current_track()
            .is_some_and(|track| track.repository_id == repository_id)
        {
            self.audio_player.stop()?;
        }

        let deleted_tracks = self.github_scanner.delete_repository(repository_id)?;

        self.repositories.retain(|repo| repo.id != repository_id);
        self.tracks
            .retain(|track| track.repository_id != repository_id);
        self.refresh_visible_tracks();
        self.select_repository_index(self.current_repository_index);
        self.select_track_index(self.current_track_index);
        self.status_message = format!(
            "Deleted {}/{} with {} tracks/cache entries",
            repository.owner, repository.name, deleted_tracks
        );

        Ok(())
    }

    fn show_error(&mut self, error: &str) {
        self.status_message = format!("Error: {}", error);
    }
}

pub async fn run(event_bus: EventBus) -> Result<(), Box<dyn std::error::Error>> {
    let database = Arc::new(DatabaseManager::new());
    let cache = Arc::new(crate::cache::CacheManager::new());
    let github_scanner = Arc::new(GitHubScanner::new(database.clone(), cache.clone()));
    let credential_store: Arc<dyn CredentialStore> = Arc::new(KeyringStore::new());
    let mut app = App::new(event_bus, database, github_scanner, cache, credential_store)?;
    app.run().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_mode_round_trips_through_str() {
        for mode in [PlaybackMode::Sequential, PlaybackMode::Shuffle] {
            let parsed: PlaybackMode = mode.as_str().parse().expect("parse known label");
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn playback_mode_rejects_unknown_value() {
        assert!("random".parse::<PlaybackMode>().is_err());
        assert!("".parse::<PlaybackMode>().is_err());
    }

    #[test]
    fn playback_mode_toggle_alternates() {
        assert_eq!(PlaybackMode::Sequential.next(), PlaybackMode::Shuffle);
        assert_eq!(PlaybackMode::Shuffle.next(), PlaybackMode::Sequential);
    }

    #[tokio::test]
    async fn playback_mode_is_loaded_from_database_on_startup() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        {
            let db = DatabaseManager::from_path(&db_path).unwrap();
            db.set_setting(PLAYBACK_MODE_SETTING_KEY, PlaybackMode::Shuffle.as_str())
                .unwrap();
        }

        let database = Arc::new(DatabaseManager::from_path(&db_path).unwrap());
        let cache = Arc::new(crate::cache::CacheManager::new());
        let github_scanner = Arc::new(GitHubScanner::new(database.clone(), cache.clone()));
        let credential_store: Arc<dyn CredentialStore> =
            Arc::new(crate::credentials::KeyringStore::new());

        let app = App::new(
            EventBus::new(),
            database,
            github_scanner,
            cache,
            credential_store,
        )
        .expect("app constructs");

        assert_eq!(app.playback_mode, PlaybackMode::Shuffle);
    }

    #[tokio::test]
    async fn playback_mode_defaults_to_sequential_when_unset() {
        let dir = tempfile::tempdir().unwrap();
        let database = Arc::new(DatabaseManager::from_path(dir.path().join("music.db")).unwrap());
        let cache = Arc::new(crate::cache::CacheManager::new());
        let github_scanner = Arc::new(GitHubScanner::new(database.clone(), cache.clone()));
        let credential_store: Arc<dyn CredentialStore> =
            Arc::new(crate::credentials::KeyringStore::new());

        let app = App::new(
            EventBus::new(),
            database,
            github_scanner,
            cache,
            credential_store,
        )
        .expect("app constructs");

        assert_eq!(app.playback_mode, PlaybackMode::Sequential);
    }
}
