//! Pure helpers used by the TUI module.
//!
//! Everything in this file is free of state — it operates only on its
//! arguments. Keeping these functions here lets us cover them with focused
//! unit tests without spinning up a terminal or a full `App` instance.

use crossterm::event::KeyEventKind;
use pinyin::ToPinyin;
use ratatui::layout::Rect;

use crate::models::{PlaybackState, Track};

pub(super) const INITIAL_STREAM_BUFFER_BYTES: u64 = 256 * 1024;
pub(super) const DEFAULT_TRACKS_PAGE_STEP: usize = 10;
pub(super) const TAB_REPOSITORIES: usize = 0;
pub(super) const TAB_TRACKS: usize = 1;
pub(super) const TAB_FAVORITES: usize = 2;
pub(super) const TAB_BLACKLIST: usize = 3;
pub(super) const TAB_COUNT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TrackListFilter {
    Tracks,
    Favorites,
    Blacklist,
}

impl TrackListFilter {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Tracks => "Tracks",
            Self::Favorites => "Favorites",
            Self::Blacklist => "Blacklist",
        }
    }

    pub(super) fn includes(self, track: &Track) -> bool {
        match self {
            Self::Tracks => !track.blacklisted,
            Self::Favorites => track.favorite && !track.blacklisted,
            Self::Blacklist => track.blacklisted,
        }
    }

    pub(super) fn can_play(self) -> bool {
        matches!(self, Self::Tracks | Self::Favorites)
    }

    pub(super) fn count(self, tracks: &[Track]) -> usize {
        tracks.iter().filter(|track| self.includes(track)).count()
    }
}

pub(super) fn track_list_filter_for_tab(tab: usize) -> Option<TrackListFilter> {
    match tab {
        TAB_TRACKS => Some(TrackListFilter::Tracks),
        TAB_FAVORITES => Some(TrackListFilter::Favorites),
        TAB_BLACKLIST => Some(TrackListFilter::Blacklist),
        _ => None,
    }
}

pub(super) fn initial_buffer_bytes(track: &Track) -> u64 {
    if track.size == 0 {
        INITIAL_STREAM_BUFFER_BYTES
    } else {
        INITIAL_STREAM_BUFFER_BYTES.min(track.size)
    }
}

pub(super) fn cache_icon(track: &Track, is_caching: bool) -> &'static str {
    if track.is_playable() {
        "✓"
    } else if is_caching {
        "↓"
    } else {
        ""
    }
}

pub(super) fn favorite_icon(track: &Track) -> &'static str {
    if track.favorite {
        "♥"
    } else {
        ""
    }
}

pub(super) fn track_page_step_for_area(area: Rect) -> usize {
    // Tracks table height minus top/bottom borders and the header row.
    usize::from(area.height.saturating_sub(3).max(1))
}

pub(super) fn normalize_search_text(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

pub(super) fn pinyin_search_text(value: &str) -> (String, String) {
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

pub(super) fn track_matches_search(track: &Track, query: &str) -> bool {
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

pub(super) fn filtered_track_indices(
    tracks: &[Track],
    query: &str,
    filter: TrackListFilter,
) -> Vec<usize> {
    tracks
        .iter()
        .enumerate()
        .filter_map(|(index, track)| {
            (filter.includes(track) && track_matches_search(track, query)).then_some(index)
        })
        .collect()
}

pub(super) fn should_handle_key_event(kind: KeyEventKind) -> bool {
    matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

pub(super) fn next_track_index(current: Option<usize>, len: usize, step: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }

    let Some(current) = current else {
        return Some(0);
    };

    Some(current.saturating_add(step.max(1)).min(len - 1))
}

pub(super) fn previous_track_index(
    current: Option<usize>,
    len: usize,
    step: usize,
) -> Option<usize> {
    if len == 0 {
        return None;
    }

    let Some(current) = current else {
        return Some(0);
    };

    Some(current.saturating_sub(step.max(1)))
}

pub(super) fn next_repository_index(current: Option<usize>, len: usize) -> Option<usize> {
    next_track_index(current, len, 1)
}

pub(super) fn previous_repository_index(current: Option<usize>, len: usize) -> Option<usize> {
    previous_track_index(current, len, 1)
}

pub(super) fn sequential_autoplay_index(current: Option<usize>, len: usize) -> Option<usize> {
    let current = current?;

    if current + 1 < len {
        Some(current + 1)
    } else {
        None
    }
}

pub(super) fn shuffle_autoplay_index(
    current: Option<usize>,
    len: usize,
    seed: &mut u64,
) -> Option<usize> {
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

pub(super) fn playback_state_symbol(playback_state: &PlaybackState) -> &'static str {
    match playback_state {
        PlaybackState::Playing => "▶",
        PlaybackState::Paused => "⏸",
        PlaybackState::Stopped => "⏹",
    }
}

pub(super) fn volume_percent(volume: f32) -> u8 {
    (volume.clamp(0.0, 1.0) * 100.0).round() as u8
}

pub(super) const MARQUEE_GAP: &str = "   ";

pub(super) fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

/// Returns a window of `text` whose display width fits within `max_width`.
/// If the full text fits, returns it unchanged; otherwise scrolls based on
/// `offset` (advanced once per tick by the caller).
pub(super) fn marquee_view(text: &str, offset: usize, max_width: usize) -> String {
    use unicode_width::UnicodeWidthChar;

    if max_width == 0 {
        return String::new();
    }
    let total: usize = text.chars().filter_map(|c| c.width()).sum();
    if total <= max_width {
        return text.to_string();
    }

    let chars: Vec<char> = text.chars().chain(MARQUEE_GAP.chars()).collect();
    let cycle = chars.len();
    let start = offset % cycle;

    let mut out = String::new();
    let mut width = 0usize;
    let mut consumed = 0usize;
    while consumed < cycle && width < max_width {
        let c = chars[(start + consumed) % cycle];
        let cw = c.width().unwrap_or(0);
        if width + cw > max_width {
            out.push(' ');
            break;
        }
        out.push(c);
        width += cw;
        consumed += 1;
    }
    out
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
    fn tab_count_matches_track_list_tabs_plus_repositories() {
        // 4 tabs in total: Repositories, Tracks, Favorites, Blacklist.
        assert_eq!(TAB_COUNT, 4);
        // Every numeric tab below TAB_COUNT is either repositories or a
        // resolvable track list filter — no holes.
        assert!(track_list_filter_for_tab(TAB_REPOSITORIES).is_none());
        for tab in 1..TAB_COUNT {
            assert!(
                track_list_filter_for_tab(tab).is_some(),
                "tab {tab} should map to a track list filter"
            );
        }
        // Anything past the last index has no mapping.
        assert!(track_list_filter_for_tab(TAB_COUNT).is_none());
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

    #[test]
    fn marquee_returns_text_unchanged_when_fits() {
        assert_eq!(marquee_view("hello", 0, 10), "hello");
        assert_eq!(marquee_view("hello", 5, 10), "hello");
    }

    #[test]
    fn marquee_scrolls_long_text_and_wraps() {
        let text = "AbCdEfGhIj";
        let first = marquee_view(text, 0, 5);
        assert_eq!(display_width(&first), 5);
        assert_eq!(first, "AbCdE");

        let second = marquee_view(text, 1, 5);
        assert_eq!(display_width(&second), 5);
        assert_eq!(second, "bCdEf");

        let wrap = marquee_view(text, text.chars().count() + MARQUEE_GAP.len(), 5);
        assert_eq!(wrap, "AbCdE");
    }

    #[test]
    fn marquee_handles_wide_chars_without_splitting() {
        let view = marquee_view("中文歌曲名称展示", 0, 5);
        assert!(display_width(&view) <= 5);
    }

    #[test]
    fn marquee_returns_empty_when_no_budget() {
        assert_eq!(marquee_view("anything", 0, 0), "");
    }
}
