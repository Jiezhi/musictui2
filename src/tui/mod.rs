use crate::audio::{prepare_streaming_decoder, AudioPlayer, StreamingDecoder};
use crate::cache::StreamingCacheState;
use crate::database::DatabaseManager;
use crate::events::EventBus;
use crate::github::GitHubScanner;
use crate::models::{PlaybackState, Repository, Track};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, terminal,
};
use pinyin::ToPinyin;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Tabs,
    },
    Frame, Terminal,
};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinHandle;

const INITIAL_STREAM_BUFFER_BYTES: u64 = 256 * 1024;
const DEFAULT_TRACKS_PAGE_STEP: usize = 10;
const TAB_REPOSITORIES: usize = 0;
const TAB_TRACKS: usize = 1;
const TAB_FAVORITES: usize = 2;
const TAB_BLACKLIST: usize = 3;
const TAB_NOW_PLAYING: usize = 4;
const TAB_COUNT: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackListFilter {
    Tracks,
    Favorites,
    Blacklist,
}

impl TrackListFilter {
    fn label(self) -> &'static str {
        match self {
            Self::Tracks => "Tracks",
            Self::Favorites => "Favorites",
            Self::Blacklist => "Blacklist",
        }
    }

    fn includes(self, track: &Track) -> bool {
        match self {
            Self::Tracks => !track.blacklisted,
            Self::Favorites => track.favorite && !track.blacklisted,
            Self::Blacklist => track.blacklisted,
        }
    }

    fn can_play(self) -> bool {
        matches!(self, Self::Tracks | Self::Favorites)
    }

    fn count(self, tracks: &[Track]) -> usize {
        tracks.iter().filter(|track| self.includes(track)).count()
    }
}

fn track_list_filter_for_tab(tab: usize) -> Option<TrackListFilter> {
    match tab {
        TAB_TRACKS => Some(TrackListFilter::Tracks),
        TAB_FAVORITES => Some(TrackListFilter::Favorites),
        TAB_BLACKLIST => Some(TrackListFilter::Blacklist),
        _ => None,
    }
}

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

struct DrawData<'a> {
    repositories: &'a Vec<Repository>,
    tracks: &'a Vec<Track>,
    visible_track_indices: &'a [usize],
    track_list_filter: TrackListFilter,
    selected_tab: usize,
    current_repository_index: Option<usize>,
    current_track_index: Option<usize>,
    current_track_row_index: Option<usize>,
    caching_track_index: Option<usize>,
    playback_state: &'a PlaybackState,
    playback_mode: PlaybackMode,
    current_track: Option<&'a Track>,
    volume: f32,
    status_message: &'a str,
    track_search_query: &'a str,
    is_searching_tracks: bool,
    show_help: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaybackMode {
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

    fn label(self) -> &'static str {
        match self {
            Self::Sequential => "Sequential",
            Self::Shuffle => "Shuffle",
        }
    }
}

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

fn initial_buffer_bytes(track: &Track) -> u64 {
    if track.size == 0 {
        INITIAL_STREAM_BUFFER_BYTES
    } else {
        INITIAL_STREAM_BUFFER_BYTES.min(track.size)
    }
}

fn cache_label(track: &Track, is_caching: bool) -> String {
    if track.is_playable() {
        "Cached".to_string()
    } else if is_caching {
        "Caching".to_string()
    } else {
        "-".to_string()
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

fn track_page_step_for_area(area: Rect) -> usize {
    // Tracks table height minus top/bottom borders and the header row.
    usize::from(area.height.saturating_sub(3).max(1))
}

fn normalize_search_text(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn pinyin_search_text(value: &str) -> (String, String) {
    let mut full_pinyin = String::new();
    let mut initials = String::new();

    for character in value.chars() {
        if let Some(pinyin) = character.to_pinyin() {
            full_pinyin.push_str(pinyin.plain());
            initials.push_str(pinyin.first_letter());
        } else {
            for lowercase in character.to_lowercase() {
                if lowercase.is_alphanumeric() {
                    full_pinyin.push(lowercase);
                    initials.push(lowercase);
                }
            }
        }
    }

    (full_pinyin, initials)
}

fn track_matches_search(track: &Track, query: &str) -> bool {
    let query = normalize_search_text(query);
    if query.is_empty() {
        return true;
    }

    let searchable_text = format!(
        "{} {} {} {}",
        track.name, track.path, track.format, track.id
    );
    if normalize_search_text(&searchable_text).contains(&query) {
        return true;
    }

    let (full_pinyin, initials) = pinyin_search_text(&searchable_text);
    full_pinyin.contains(&query) || initials.contains(&query)
}

fn filtered_track_indices(tracks: &[Track], query: &str, filter: TrackListFilter) -> Vec<usize> {
    tracks
        .iter()
        .enumerate()
        .filter_map(|(index, track)| {
            (filter.includes(track) && track_matches_search(track, query)).then_some(index)
        })
        .collect()
}

fn should_handle_key_event(kind: KeyEventKind) -> bool {
    matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

fn next_track_index(current: Option<usize>, len: usize, step: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }

    let Some(current) = current else {
        return Some(0);
    };

    Some(current.saturating_add(step.max(1)).min(len - 1))
}

fn previous_track_index(current: Option<usize>, len: usize, step: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }

    let Some(current) = current else {
        return Some(0);
    };

    Some(current.saturating_sub(step.max(1)))
}

fn next_repository_index(current: Option<usize>, len: usize) -> Option<usize> {
    next_track_index(current, len, 1)
}

fn previous_repository_index(current: Option<usize>, len: usize) -> Option<usize> {
    previous_track_index(current, len, 1)
}

fn sequential_autoplay_index(current: Option<usize>, len: usize) -> Option<usize> {
    let current = current?;

    if current + 1 < len {
        Some(current + 1)
    } else {
        None
    }
}

fn shuffle_autoplay_index(current: Option<usize>, len: usize, seed: &mut u64) -> Option<usize> {
    if len == 0 {
        return None;
    }

    if len == 1 {
        return Some(0);
    }

    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    let mut index = ((*seed >> 32) as usize) % len;

    if Some(index) == current {
        index = (index + 1) % len;
    }

    Some(index)
}

fn tracks_table(data: &DrawData<'_>) -> Table<'static> {
    let rows: Vec<Row<'static>> = data
        .visible_track_indices
        .iter()
        .filter_map(|&track_index| {
            data.tracks
                .get(track_index)
                .map(|track| (track_index, track))
        })
        .map(|(track_index, track)| {
            let mut row = Row::new(vec![
                if track.favorite { "*" } else { "" }.to_string(),
                track.name.clone(),
                track.format.clone(),
                format!("{}MB", track.size / 1024 / 1024),
                cache_label(track, Some(track_index) == data.caching_track_index),
            ]);
            if Some(track_index) == data.current_track_index {
                row = row.style(Style::default().fg(Color::Yellow));
            }
            row
        })
        .collect();

    let list_total = data.track_list_filter.count(data.tracks);
    let title = if data.track_search_query.is_empty() && !data.is_searching_tracks {
        data.track_list_filter.label().to_string()
    } else {
        let cursor = if data.is_searching_tracks { "_" } else { "" };
        format!(
            "{} | Search: {}{} ({}/{})",
            data.track_list_filter.label(),
            data.track_search_query,
            cursor,
            data.visible_track_indices.len(),
            list_total
        )
    };

    Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Percentage(50),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ],
    )
    .block(Block::default().title(title).borders(Borders::ALL))
    .header(
        Row::new(vec!["Fav", "Name", "Format", "Size", "Cache"])
            .style(Style::default().fg(Color::Cyan)),
    )
    .highlight_style(Style::default().fg(Color::Yellow))
}

fn playback_state_label(playback_state: &PlaybackState) -> &'static str {
    match playback_state {
        PlaybackState::Playing => "Playing",
        PlaybackState::Paused => "Paused",
        PlaybackState::Stopped => "Stopped",
    }
}

fn volume_percent(volume: f32) -> u8 {
    (volume.clamp(0.0, 1.0) * 100.0).round() as u8
}

fn footer_text(data: &DrawData<'_>) -> String {
    let search_status = if data.is_searching_tracks {
        format!(" | Search: /{}_", data.track_search_query)
    } else if data.track_search_query.is_empty() {
        String::new()
    } else {
        format!(" | Search: {}", data.track_search_query)
    };

    format!(
        "{} | Playback: {} | Mode: {} | Vol: {}%{} | ?: Shortcuts",
        data.status_message,
        playback_state_label(data.playback_state),
        data.playback_mode.label(),
        volume_percent(data.volume),
        search_status
    )
}

fn help_text() -> &'static str {
    "?: Close shortcuts\n\
Esc: Close shortcuts / cancel delete\n\
Tab or n: Next tab\n\
Shift+Tab or p: Previous tab\n\
Up/Down or k/j: Move selection\n\
PageUp/PageDown or Ctrl+B/Ctrl+F: Page track lists\n\
/: Search current track list by name, pinyin, or initials\n\
Backspace: Delete search text while searching\n\
Enter: Keep search while searching\n\
Enter: Play selected track\n\
f: Toggle favorite on selected track\n\
x: Toggle blacklist on selected track\n\
Space: Play or pause\n\
m: Toggle playback mode\n\
, / .: Previous / next track\n\
d: Delete selected repository (press twice)\n\
+ / -: Volume\n\
q: Quit"
}

fn help_popup_area(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(76).max(area.width.min(20));
    let height = area
        .height
        .saturating_sub(2)
        .min(18)
        .max(area.height.min(3));
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;

    Rect {
        x,
        y,
        width,
        height,
    }
}

fn render_help_popup(f: &mut Frame<'_>, area: Rect) {
    let popup_area = help_popup_area(area);
    let popup = Paragraph::new(help_text())
        .block(Block::default().title("Shortcuts").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .wrap(ratatui::widgets::Wrap { trim: false });

    f.render_widget(Clear, popup_area);
    f.render_widget(popup, popup_area);
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
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let terminal = None;
        let audio_player = AudioPlayer::with_github_scanner(github_scanner.clone())?;
        let shuffle_seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0x05ee_d5ee_dd15_ca11_u64);

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
            playback_mode: PlaybackMode::Sequential,
            shuffle_seed,
            pending_repository_delete: None,
            event_bus,
            database,
            github_scanner,
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
                current_track: self.audio_player.get_current_track(),
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
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(3),
                            Constraint::Length(1),
                            Constraint::Min(0),
                            Constraint::Length(1),
                        ])
                        .split(f.area());

                    *tracks_page_step = track_page_step_for_area(chunks[2]);

                    let block = Block::default()
                        .title("Musictui2 - Terminal Music Player")
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::Cyan));

                    let active_total = draw_data.track_list_filter.count(draw_data.tracks);
                    let favorites_total = TrackListFilter::Favorites.count(draw_data.tracks);
                    let blacklisted_total = TrackListFilter::Blacklist.count(draw_data.tracks);
                    let text = if draw_data.track_search_query.is_empty() {
                        format!(
                            "Repositories: {} | Tracks: {} | Favorites: {} | Blacklist: {}",
                            draw_data.repositories.len(),
                            TrackListFilter::Tracks.count(draw_data.tracks),
                            favorites_total,
                            blacklisted_total
                        )
                    } else {
                        format!(
                            "Repositories: {} | {}: {}/{} | Favorites: {} | Blacklist: {}",
                            draw_data.repositories.len(),
                            draw_data.track_list_filter.label(),
                            draw_data.visible_track_indices.len(),
                            active_total,
                            favorites_total,
                            blacklisted_total
                        )
                    };

                    let header = Paragraph::new(text)
                        .block(block)
                        .style(Style::default().fg(Color::White));
                    f.render_widget(header, chunks[0]);

                    let titles = [
                        "Repositories",
                        "Tracks",
                        "Favorites",
                        "Blacklist",
                        "Now Playing",
                    ];
                    let tabs_widget = Tabs::new(titles.iter().copied())
                        .block(Block::default().borders(Borders::NONE))
                        .select(draw_data.selected_tab)
                        .style(Style::default().fg(Color::White))
                        .highlight_style(Style::default().fg(Color::Yellow));
                    f.render_widget(tabs_widget, chunks[1]);

                    match draw_data.selected_tab {
                        0 => {
                            let items: Vec<ListItem> = draw_data
                                .repositories
                                .iter()
                                .map(|repo| {
                                    ListItem::new(format!("{} - {}", repo.name, repo.owner))
                                })
                                .collect();
                            let list = List::new(items)
                                .block(Block::default().title("Repositories").borders(Borders::ALL))
                                .highlight_style(Style::default().fg(Color::Yellow))
                                .highlight_symbol("> ");
                            repositories_list_state.select(draw_data.current_repository_index);
                            f.render_stateful_widget(list, chunks[2], repositories_list_state);
                        }
                        TAB_TRACKS | TAB_FAVORITES | TAB_BLACKLIST => {
                            tracks_table_state.select(draw_data.current_track_row_index);
                            f.render_stateful_widget(
                                tracks_table(&draw_data),
                                chunks[2],
                                tracks_table_state,
                            );
                        }
                        TAB_NOW_PLAYING => {
                            let status = match draw_data.playback_state {
                                PlaybackState::Playing => "Playing",
                                PlaybackState::Paused => "Paused",
                                PlaybackState::Stopped => "Stopped",
                            };
                            let content = if let Some(track) = draw_data.current_track {
                                format!("{}\n{}\nStatus: {}", track.name, track.path, status)
                            } else {
                                "No track playing".to_string()
                            };

                            let paragraph = Paragraph::new(content)
                                .block(Block::default().title("Now Playing").borders(Borders::ALL))
                                .wrap(ratatui::widgets::Wrap { trim: true });
                            f.render_widget(paragraph, chunks[2]);
                        }
                        _ => {}
                    }

                    let footer = Paragraph::new(footer_text(&draw_data))
                        .style(Style::default().fg(Color::Gray));
                    f.render_widget(footer, chunks[3]);

                    if draw_data.show_help {
                        render_help_popup(f, f.area());
                    }
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
                self.status_message = format!("Playback mode: {}", self.playback_mode.label());
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
                } else {
                    if let Some(index) = self.playing_track_index() {
                        self.queue_track(index)?;
                    }
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

            let streaming_download = self
                .github_scanner
                .start_streaming_download(track.clone())?;
            let decoder_cache_path = streaming_download.cache_path.clone();
            let decoder_cache_state = streaming_download.state.clone();
            let format = track.format.clone();
            let initial_buffer_bytes = initial_buffer_bytes(&track);
            let decoder_handle = tokio::task::spawn_blocking(move || {
                prepare_streaming_decoder(
                    decoder_cache_path,
                    decoder_cache_state,
                    format,
                    initial_buffer_bytes,
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

    #[allow(dead_code)]
    fn draw_with_data(&self, f: &mut Frame<'_>, data: DrawData<'_>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Length(1), // Tabs
                Constraint::Min(0),    // Main content
                Constraint::Length(1), // Footer
            ])
            .split(f.area());

        // Draw header
        self.draw_header(f, chunks[0], &data);

        // Draw tabs
        self.draw_tabs(f, chunks[1], &data);

        // Draw main content based on selected tab
        match data.selected_tab {
            TAB_REPOSITORIES => self.draw_repositories(f, chunks[2], &data),
            TAB_TRACKS | TAB_FAVORITES | TAB_BLACKLIST => self.draw_tracks(f, chunks[2], &data),
            TAB_NOW_PLAYING => self.draw_now_playing(f, chunks[2], &data),
            _ => {}
        }

        // Draw footer
        self.draw_footer(f, chunks[3], &data);

        if data.show_help {
            render_help_popup(f, f.area());
        }
    }

    #[allow(dead_code)]
    fn draw_header(&self, f: &mut Frame, area: ratatui::layout::Rect, _data: &DrawData<'_>) {
        let block = Block::default()
            .title("Musictui2 - Terminal Music Player")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan));

        let text = format!(
            "Repositories: {} | {}: {}/{} | Favorites: {} | Blacklist: {}",
            self.repositories.len(),
            self.current_track_list_filter().label(),
            self.visible_track_indices.len(),
            self.current_track_list_filter().count(&self.tracks),
            TrackListFilter::Favorites.count(&self.tracks),
            TrackListFilter::Blacklist.count(&self.tracks)
        );
        let paragraph = Paragraph::new(text)
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(paragraph, area);
    }

    #[allow(dead_code)]
    fn draw_tabs(&self, f: &mut Frame, area: ratatui::layout::Rect, _data: &DrawData<'_>) {
        let titles = [
            "Repositories",
            "Tracks",
            "Favorites",
            "Blacklist",
            "Now Playing",
        ];

        let tabs_widget = Tabs::new(titles.iter().copied())
            .block(Block::default().borders(Borders::NONE))
            .select(self.selected_tab)
            .style(Style::default().fg(Color::White))
            .highlight_style(Style::default().fg(Color::Yellow));

        f.render_widget(tabs_widget, area);
    }

    #[allow(dead_code)]
    fn draw_repositories(&self, f: &mut Frame, area: ratatui::layout::Rect, data: &DrawData<'_>) {
        let block = Block::default().title("Repositories").borders(Borders::ALL);

        let items: Vec<ListItem> = data
            .repositories
            .iter()
            .map(|repo| ListItem::new(format!("{} - {}", repo.name, repo.owner)))
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().fg(Color::Yellow));

        f.render_widget(list, area);
    }

    #[allow(dead_code)]
    fn draw_tracks(&self, f: &mut Frame, area: ratatui::layout::Rect, data: &DrawData<'_>) {
        f.render_widget(tracks_table(data), area);
    }

    #[allow(dead_code)]
    fn draw_now_playing(&self, f: &mut Frame, area: ratatui::layout::Rect, data: &DrawData<'_>) {
        let block = Block::default().title("Now Playing").borders(Borders::ALL);

        let content = if let Some(track) = data.current_track {
            let status = match data.playback_state {
                PlaybackState::Playing => "Playing",
                PlaybackState::Paused => "Paused",
                PlaybackState::Stopped => "Stopped",
            };

            format!("{}\n{}\nStatus: {}", track.name, track.path, status)
        } else {
            "No track playing".to_string()
        };

        let paragraph = Paragraph::new(content)
            .block(block)
            .wrap(ratatui::widgets::Wrap { trim: true });

        f.render_widget(paragraph, area);
    }

    #[allow(dead_code)]
    fn draw_footer(&self, f: &mut Frame, area: ratatui::layout::Rect, data: &DrawData<'_>) {
        let paragraph = Paragraph::new(footer_text(data)).style(Style::default().fg(Color::Gray));

        f.render_widget(paragraph, area);
    }

    fn show_error(&mut self, error: &str) {
        self.status_message = format!("Error: {}", error);
    }
}

pub async fn run(event_bus: EventBus) -> Result<(), Box<dyn std::error::Error>> {
    let database = Arc::new(DatabaseManager::new());
    let cache = Arc::new(crate::cache::CacheManager::new());
    let github_scanner = Arc::new(GitHubScanner::new(database.clone(), cache));
    let mut app = App::new(event_bus, database, github_scanner)?;
    app.run().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_track(id: i64, name: &str, path: &str) -> Track {
        Track {
            id,
            repository_id: 1,
            path: path.to_string(),
            name: name.to_string(),
            format: "mp3".to_string(),
            size: 1024,
            duration: None,
            url: format!("https://example.com/{name}"),
            local_path: None,
            downloaded: false,
            discovered_at: chrono::Utc::now(),
            favorite: false,
            blacklisted: false,
        }
    }

    #[test]
    fn track_page_step_matches_visible_table_rows() {
        assert_eq!(track_page_step_for_area(Rect::new(0, 0, 80, 20)), 17);
        assert_eq!(track_page_step_for_area(Rect::new(0, 0, 80, 2)), 1);
    }

    #[test]
    fn key_event_filter_ignores_windows_key_release_events() {
        assert!(should_handle_key_event(KeyEventKind::Press));
        assert!(should_handle_key_event(KeyEventKind::Repeat));
        assert!(!should_handle_key_event(KeyEventKind::Release));
    }

    #[test]
    fn next_track_index_pages_and_clamps() {
        assert_eq!(next_track_index(None, 10, 5), Some(0));
        assert_eq!(next_track_index(Some(2), 10, 5), Some(7));
        assert_eq!(next_track_index(Some(8), 10, 5), Some(9));
        assert_eq!(next_track_index(Some(2), 10, 0), Some(3));
        assert_eq!(next_track_index(Some(0), 0, 5), None);
    }

    #[test]
    fn previous_track_index_pages_and_clamps() {
        assert_eq!(previous_track_index(None, 10, 5), Some(0));
        assert_eq!(previous_track_index(Some(7), 10, 5), Some(2));
        assert_eq!(previous_track_index(Some(2), 10, 5), Some(0));
        assert_eq!(previous_track_index(Some(2), 10, 0), Some(1));
        assert_eq!(previous_track_index(Some(0), 0, 5), None);
    }

    #[test]
    fn sequential_autoplay_stops_at_end() {
        assert_eq!(sequential_autoplay_index(Some(0), 3), Some(1));
        assert_eq!(sequential_autoplay_index(Some(2), 3), None);
        assert_eq!(sequential_autoplay_index(None, 3), None);
    }

    #[test]
    fn shuffle_autoplay_avoids_current_track_when_possible() {
        let mut seed = 1;
        for current in 0..5 {
            assert_ne!(
                shuffle_autoplay_index(Some(current), 5, &mut seed),
                Some(current)
            );
        }
        assert_eq!(shuffle_autoplay_index(Some(0), 1, &mut seed), Some(0));
        assert_eq!(shuffle_autoplay_index(Some(0), 0, &mut seed), None);
    }

    #[test]
    fn track_search_matches_text_pinyin_and_initials() {
        let track = test_track(1, "告白气球.mp3", "albums/周杰伦/告白气球.mp3");

        assert!(track_matches_search(&track, "告白"));
        assert!(track_matches_search(&track, "gaobai"));
        assert!(track_matches_search(&track, "gbqq"));
        assert!(track_matches_search(&track, "zhoujielun"));
        assert!(track_matches_search(&track, "zjl"));
        assert!(!track_matches_search(&track, "晴天"));
    }

    #[test]
    fn filtered_track_indices_preserve_original_track_indices() {
        let tracks = vec![
            test_track(1, "晴天.mp3", "albums/周杰伦/晴天.mp3"),
            test_track(2, "告白气球.mp3", "albums/周杰伦/告白气球.mp3"),
            test_track(3, "Blue.mp3", "albums/English/Blue.mp3"),
        ];

        assert_eq!(
            filtered_track_indices(&tracks, "zjl", TrackListFilter::Tracks),
            vec![0, 1]
        );
        assert_eq!(
            filtered_track_indices(&tracks, "gbqq", TrackListFilter::Tracks),
            vec![1]
        );
        assert_eq!(
            filtered_track_indices(&tracks, "blue", TrackListFilter::Tracks),
            vec![2]
        );
    }

    #[test]
    fn filtered_track_indices_respect_track_list_filter() {
        let mut favorite = test_track(1, "Favorite.mp3", "favorite.mp3");
        favorite.favorite = true;
        let mut blacklisted = test_track(2, "Blocked.mp3", "blocked.mp3");
        blacklisted.favorite = true;
        blacklisted.blacklisted = true;
        let normal = test_track(3, "Normal.mp3", "normal.mp3");
        let tracks = vec![favorite, blacklisted, normal];

        assert_eq!(
            filtered_track_indices(&tracks, "", TrackListFilter::Tracks),
            vec![0, 2]
        );
        assert_eq!(
            filtered_track_indices(&tracks, "", TrackListFilter::Favorites),
            vec![0]
        );
        assert_eq!(
            filtered_track_indices(&tracks, "", TrackListFilter::Blacklist),
            vec![1]
        );
    }
}
