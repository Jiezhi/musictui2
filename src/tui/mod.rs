use std::io;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph, Row, Table, Tabs},
    Frame, Terminal,
};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute, terminal,
};
use crate::events::EventBus;
use crate::models::{Track, Repository, PlaybackState};
use crate::database::DatabaseManager;
use crate::audio::AudioPlayer;
use crate::github::GitHubScanner;
use std::sync::Arc;

struct DrawData<'a> {
    repositories: &'a Vec<Repository>,
    tracks: &'a Vec<Track>,
    selected_tab: usize,
    current_track_index: Option<usize>,
    playback_state: &'a PlaybackState,
    current_track: Option<&'a Track>,
}

pub struct App {
    terminal: Option<Terminal<CrosstermBackend<io::Stdout>>>,
    selected_tab: usize,
    repositories: Vec<Repository>,
    tracks: Vec<Track>,
    audio_player: AudioPlayer,
    current_track_index: Option<usize>,
    #[allow(dead_code)]
    event_bus: EventBus,
    database: Arc<DatabaseManager>,
    #[allow(dead_code)]
    github_scanner: Arc<GitHubScanner>,
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

        Ok(Self {
            terminal,
            selected_tab: 0,
            repositories: Vec::new(),
            tracks: Vec::new(),
            audio_player,
            current_track_index: None,
            event_bus,
            database,
            github_scanner,
            should_quit: false,
        })
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut stdout = io::stdout();
        terminal::enable_raw_mode()?;
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
                    if let Err(err) = self.handle_key_event(key).await {
                        self.show_error(&err.to_string());
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                self.on_tick();
                last_tick = std::time::Instant::now();
            }

            let draw_data = DrawData {
                repositories: &self.repositories,
                tracks: &self.tracks,
                selected_tab: self.selected_tab,
                current_track_index: self.current_track_index,
                playback_state: self.audio_player.get_playback_state(),
                current_track: self.audio_player.get_current_track(),
            };

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

                    let block = Block::default()
                        .title("Musictui2 - Terminal Music Player")
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::Cyan));

                    let text = format!(
                        "Repositories: {} | Tracks: {}",
                        draw_data.repositories.len(),
                        draw_data.tracks.len()
                    );

                    let header = Paragraph::new(text)
                        .block(block)
                        .style(Style::default().fg(Color::White));
                    f.render_widget(header, chunks[0]);

                    let titles = ["Repositories", "Tracks", "Now Playing"];
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
                                .map(|repo| ListItem::new(format!("{} - {}", repo.name, repo.owner)))
                                .collect();
                            let list = List::new(items).block(
                                Block::default().title("Repositories").borders(Borders::ALL),
                            );
                            f.render_widget(list, chunks[2]);
                        }
                        1 => {
                            let rows: Vec<Row> = draw_data
                                .tracks
                                .iter()
                                .enumerate()
                                .map(|(i, track)| {
                                    let mut row = Row::new(vec![
                                        track.name.clone(),
                                        track.format.clone(),
                                        format!("{}MB", track.size / 1024 / 1024),
                                    ]);
                                    if Some(i) == draw_data.current_track_index {
                                        row = row.style(Style::default().fg(Color::Yellow));
                                    }
                                    row
                                })
                                .collect();

                            let table = Table::new(
                                rows,
                                [
                                    Constraint::Percentage(60),
                                    Constraint::Percentage(20),
                                    Constraint::Percentage(20),
                                ],
                            )
                            .block(Block::default().title("Tracks").borders(Borders::ALL))
                            .header(
                                Row::new(vec!["Name", "Format", "Size"])
                                    .style(Style::default().fg(Color::Cyan)),
                            );
                            f.render_widget(table, chunks[2]);
                        }
                        _ => {
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
                    }

                    let help = Paragraph::new(
                        "Tab: Switch | ↑↓/jk: Navigate | Enter: Play | Space: Play/Pause | +/-: Volume | q: Quit",
                    )
                    .style(Style::default().fg(Color::Gray));
                    f.render_widget(help, chunks[3]);
                })?;
            }
        }

        if let Some(terminal) = &mut self.terminal {
            terminal::disable_raw_mode()?;
            execute!(
                terminal.backend_mut(),
                terminal::LeaveAlternateScreen,
                crossterm::event::DisableMouseCapture
            )?;
        }
        self.terminal = None;

        Ok(())
    }

    fn load_data(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.repositories = self.database.get_repositories()?;
        if let Some(repo) = self.repositories.first() {
            self.tracks = self.database.get_tracks_by_repo(repo.id)?;
        }
        Ok(())
    }

    async fn handle_key_event(&mut self, key: KeyEvent) -> Result<(), Box<dyn std::error::Error>> {
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
            },
            KeyCode::Char('n') => {
                self.selected_tab = (self.selected_tab + 1) % 3;
            },
            KeyCode::Char('p') => {
                self.selected_tab = (self.selected_tab + 3 - 1) % 3;
            },
            KeyCode::Tab => {
                self.selected_tab = (self.selected_tab + 1) % 3;
            },
            KeyCode::BackTab => {
                self.selected_tab = (self.selected_tab + 3 - 1) % 3;
            },
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_tab == 0 {
                    // Navigate repositories - not implemented yet
                } else if self.selected_tab == 1 {
                    // Navigate tracks up
                    if let Some(current) = self.current_track_index {
                        if current > 0 {
                            self.current_track_index = Some(current - 1);
                        }
                    } else if !self.tracks.is_empty() {
                        self.current_track_index = Some(0);
                    }
                }
            },
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected_tab == 0 {
                    // Navigate repositories - not implemented yet
                } else if self.selected_tab == 1 {
                    // Navigate tracks down
                    if let Some(current) = self.current_track_index {
                        if current < self.tracks.len() - 1 {
                            self.current_track_index = Some(current + 1);
                        }
                    } else if !self.tracks.is_empty() {
                        self.current_track_index = Some(0);
                    }
                }
            },
            KeyCode::Enter => {
                if self.selected_tab == 1 && !self.tracks.is_empty() {
                    if let Some(index) = self.current_track_index {
                        self.play_track(index).await?;
                    }
                }
            },
            KeyCode::Char(' ') => {
                if self.audio_player.is_playing() {
                    self.audio_player.pause()?;
                } else {
                    if let Some(index) = self.current_track_index {
                        self.play_track(index).await?;
                    }
                }
            },
            KeyCode::Char('+') => {
                let new_volume = (self.audio_player.get_volume() + 0.1).min(1.0);
                self.audio_player.set_volume(new_volume)?;
            },
            KeyCode::Char('-') => {
                let new_volume = (self.audio_player.get_volume() - 0.1).max(0.0);
                self.audio_player.set_volume(new_volume)?;
            },
            _ => {}
        }

        Ok(())
    }

    async fn play_track(&mut self, index: usize) -> Result<(), Box<dyn std::error::Error>> {
        if index < self.tracks.len() {
            let track = &self.tracks[index];
            self.audio_player.load_track(track.clone()).await?;
            self.audio_player.play()?;
            self.current_track_index = Some(index);
        }
        Ok(())
    }

    fn on_tick(&mut self) {
        // Update UI based on current state
        if self.audio_player.is_playing() {
            // Could update progress bar here
        }
    }

    #[allow(dead_code)]
    fn draw_with_data(&self, f: &mut Frame<'_>, data: DrawData<'_>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Header
                Constraint::Length(1),  // Tabs
                Constraint::Min(0),      // Main content
                Constraint::Length(1),  // Footer
            ])
            .split(f.area());

        // Draw header
        self.draw_header(f, chunks[0], &data);

        // Draw tabs
        self.draw_tabs(f, chunks[1], &data);

        // Draw main content based on selected tab
        match data.selected_tab {
            0 => self.draw_repositories(f, chunks[2], &data),
            1 => self.draw_tracks(f, chunks[2], &data),
            2 => self.draw_now_playing(f, chunks[2], &data),
            _ => {}
        }

        // Draw footer
        self.draw_footer(f, chunks[3], &data);
    }

    #[allow(dead_code)]
    fn draw_header(&self, f: &mut Frame, area: ratatui::layout::Rect, _data: &DrawData<'_>) {
        let block = Block::default()
            .title("Musictui2 - Terminal Music Player")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan));

        let text = format!("Repositories: {} | Tracks: {}", self.repositories.len(), self.tracks.len());
        let paragraph = Paragraph::new(text)
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(paragraph, area);
    }

    #[allow(dead_code)]
    fn draw_tabs(&self, f: &mut Frame, area: ratatui::layout::Rect, _data: &DrawData<'_>) {
        let titles = ["Repositories", "Tracks", "Now Playing"];

        let tabs_widget = Tabs::new(titles.iter().copied())
            .block(Block::default().borders(Borders::NONE))
            .select(self.selected_tab)
            .style(Style::default().fg(Color::White))
            .highlight_style(Style::default().fg(Color::Yellow));

        f.render_widget(tabs_widget, area);
    }

    #[allow(dead_code)]
    fn draw_repositories(&self, f: &mut Frame, area: ratatui::layout::Rect, data: &DrawData<'_>) {
        let block = Block::default()
            .title("Repositories")
            .borders(Borders::ALL);

        let items: Vec<ListItem> = data.repositories
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
        let block = Block::default()
            .title("Tracks")
            .borders(Borders::ALL);

        let rows: Vec<Row> = data.tracks
            .iter()
            .enumerate()
            .map(|(i, track)| {
                let mut row = Row::new(vec![
                    track.name.clone(),
                    track.format.clone(),
                    format!("{}MB", track.size / 1024 / 1024),
                ]);

                if Some(i) == data.current_track_index {
                    row = row.style(Style::default().fg(Color::Yellow));
                }

                row
            })
            .collect();

        let table = Table::new(rows, &[
                Constraint::Percentage(60),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
            ])
            .block(block)
            .header(Row::new(vec!["Name", "Format", "Size"]).style(Style::default().fg(Color::Cyan)));

        f.render_widget(table, area);
    }

    #[allow(dead_code)]
    fn draw_now_playing(&self, f: &mut Frame, area: ratatui::layout::Rect, data: &DrawData<'_>) {
        let block = Block::default()
            .title("Now Playing")
            .borders(Borders::ALL);

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
    fn draw_footer(&self, f: &mut Frame, area: ratatui::layout::Rect, _data: &DrawData<'_>) {
        let help_text = "Tab: Switch | ↑↓/jk: Navigate | Enter: Play | Space: Play/Pause | +/-: Volume | q: Quit";
        let paragraph = Paragraph::new(help_text)
            .style(Style::default().fg(Color::Gray));

        f.render_widget(paragraph, area);
    }

    fn show_error(&self, error: &str) {
        // In a real implementation, this would show an error message in the UI
        eprintln!("Error: {}", error);
    }
}

pub async fn run(event_bus: EventBus) -> Result<(), Box<dyn std::error::Error>> {
    let database = Arc::new(DatabaseManager::new());
    let cache = Arc::new(crate::cache::CacheManager::new());
    let github_scanner = Arc::new(GitHubScanner::new(database.clone(), cache));
    let mut app = App::new(event_bus, database, github_scanner)?;
    app.run().await
}