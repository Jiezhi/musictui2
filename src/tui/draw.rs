//! Drawing helpers for the TUI.
//!
//! Keeping all `ratatui` widget construction in a single module lets the rest
//! of the app focus on state transitions. The entry point is
//! [`render_frame`], which is invoked from the main event loop with a fully
//! constructed [`DrawData`] snapshot.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Tabs,
};
use ratatui::Frame;

use crate::models::{PlaybackState, Repository, Track};

use super::util::{
    display_width, marquee_view, playback_state_symbol, track_page_step_for_area, volume_percent,
    TrackListFilter, TAB_BLACKLIST, TAB_FAVORITES, TAB_TRACKS,
};
use super::PlaybackMode;

pub(super) struct DrawData<'a> {
    pub(super) repositories: &'a Vec<Repository>,
    pub(super) tracks: &'a Vec<Track>,
    pub(super) visible_track_indices: &'a [usize],
    pub(super) track_list_filter: TrackListFilter,
    pub(super) selected_tab: usize,
    pub(super) current_repository_index: Option<usize>,
    pub(super) current_track_index: Option<usize>,
    pub(super) current_track_row_index: Option<usize>,
    pub(super) caching_track_index: Option<usize>,
    pub(super) playback_state: &'a PlaybackState,
    pub(super) playback_mode: PlaybackMode,
    pub(super) volume: f32,
    pub(super) status_message: &'a str,
    pub(super) marquee_offset: usize,
    pub(super) track_search_query: &'a str,
    pub(super) is_searching_tracks: bool,
    pub(super) show_help: bool,
}

/// Renders the full TUI for one tick.
pub(super) fn render_frame(
    f: &mut Frame<'_>,
    data: &DrawData<'_>,
    repositories_list_state: &mut ListState,
    tracks_table_state: &mut TableState,
    tracks_page_step: &mut usize,
) {
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

    render_header(f, chunks[0], data);
    render_tabs(f, chunks[1], data);
    render_main_content(
        f,
        chunks[2],
        data,
        repositories_list_state,
        tracks_table_state,
    );
    render_footer(f, chunks[3], data);

    if data.show_help {
        render_help_popup(f, f.area());
    }
}

fn render_header(f: &mut Frame<'_>, area: Rect, data: &DrawData<'_>) {
    let block = Block::default()
        .title("Musictui2 - Terminal Music Player")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));

    let active_total = data.track_list_filter.count(data.tracks);
    let favorites_total = TrackListFilter::Favorites.count(data.tracks);
    let blacklisted_total = TrackListFilter::Blacklist.count(data.tracks);
    let text = if data.track_search_query.is_empty() {
        format!(
            "Repositories: {} | Tracks: {} | Favorites: {} | Blacklist: {}",
            data.repositories.len(),
            TrackListFilter::Tracks.count(data.tracks),
            favorites_total,
            blacklisted_total
        )
    } else {
        format!(
            "Repositories: {} | {}: {}/{} | Favorites: {} | Blacklist: {}",
            data.repositories.len(),
            data.track_list_filter.label(),
            data.visible_track_indices.len(),
            active_total,
            favorites_total,
            blacklisted_total
        )
    };

    let header = Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(Color::White));
    f.render_widget(header, area);
}

fn render_tabs(f: &mut Frame<'_>, area: Rect, data: &DrawData<'_>) {
    let titles = ["Repositories", "Tracks", "Favorites", "Blacklist"];
    let tabs_widget = Tabs::new(titles.iter().copied())
        .block(Block::default().borders(Borders::NONE))
        .select(data.selected_tab)
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Yellow));
    f.render_widget(tabs_widget, area);
}

fn render_main_content(
    f: &mut Frame<'_>,
    area: Rect,
    data: &DrawData<'_>,
    repositories_list_state: &mut ListState,
    tracks_table_state: &mut TableState,
) {
    match data.selected_tab {
        0 => {
            let items: Vec<ListItem> = data
                .repositories
                .iter()
                .map(|repo| ListItem::new(format!("{} - {}", repo.name, repo.owner)))
                .collect();
            let list = List::new(items)
                .block(Block::default().title("Repositories").borders(Borders::ALL))
                .highlight_style(Style::default().fg(Color::Yellow))
                .highlight_symbol("> ");
            repositories_list_state.select(data.current_repository_index);
            f.render_stateful_widget(list, area, repositories_list_state);
        }
        TAB_TRACKS | TAB_FAVORITES | TAB_BLACKLIST => {
            tracks_table_state.select(data.current_track_row_index);
            f.render_stateful_widget(tracks_table(data), area, tracks_table_state);
        }
        _ => {}
    }
}

fn render_footer(f: &mut Frame<'_>, area: Rect, data: &DrawData<'_>) {
    let footer =
        Paragraph::new(footer_text(data, area.width as usize)).style(Style::default().fg(Color::Gray));
    f.render_widget(footer, area);
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
                super::util::cache_label(track, Some(track_index) == data.caching_track_index),
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

fn footer_text(data: &DrawData<'_>, max_width: usize) -> String {
    let search_part = if data.is_searching_tracks {
        format!(" │ /{}_", data.track_search_query)
    } else if data.track_search_query.is_empty() {
        String::new()
    } else {
        format!(" │ /{}", data.track_search_query)
    };

    let suffix = format!(
        " │ {} │ {} │ {}% │ ?{}",
        playback_state_symbol(data.playback_state),
        data.playback_mode.symbol(),
        volume_percent(data.volume),
        search_part,
    );
    let suffix_width = display_width(&suffix);
    let status_budget = max_width.saturating_sub(suffix_width);

    if status_budget == 0 || data.status_message.is_empty() {
        return if data.status_message.is_empty() {
            suffix.trim_start_matches(" │ ").to_string()
        } else {
            data.status_message.to_string()
        };
    }

    let displayed = marquee_view(data.status_message, data.marquee_offset, status_budget);
    format!("{}  {}", displayed, suffix.trim_start_matches(" │ "))
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
