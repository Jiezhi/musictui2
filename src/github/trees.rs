//! Pure helpers for the GitHub Git Trees API.
//!
//! These functions are intentionally side-effect-free so they can be exercised
//! by unit tests without any network access. The HTTP layer in `super` calls
//! into [`tree_to_tracks`] after deserializing the API response.

use chrono::Utc;
use serde::Deserialize;

use crate::models::Track;

#[derive(Debug, Deserialize)]
pub struct TreeResponse {
    /// Tree SHA returned by the API. Not used by current callers but part of
    /// the documented response shape.
    #[allow(dead_code)]
    pub sha: String,
    pub tree: Vec<TreeEntry>,
    /// GitHub returns `true` when the recursive tree was truncated. We expose
    /// this so callers can fall back to the contents API for huge repos.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Deserialize)]
pub struct TreeEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub size: Option<u64>,
}

/// GitHub repository metadata used to discover the default branch.
#[derive(Debug, Deserialize)]
pub struct RepoMeta {
    pub default_branch: String,
}

/// Transforms a flat recursive tree listing into the project's `Track` model,
/// keeping only known audio formats. URLs are constructed against the
/// `raw.githubusercontent.com` host so playback can stream the file directly.
pub fn tree_to_tracks(
    entries: &[TreeEntry],
    owner: &str,
    repo_name: &str,
    branch_ref: &str,
    repository_id: i64,
) -> Vec<Track> {
    entries
        .iter()
        .filter(|entry| entry.kind == "blob")
        .filter_map(|entry| {
            let name = entry
                .path
                .rsplit('/')
                .next()
                .filter(|segment| !segment.is_empty())?
                .to_string();
            let ext = std::path::Path::new(&name)
                .extension()?
                .to_string_lossy()
                .to_lowercase();
            if !is_audio_format(&ext) {
                return None;
            }
            let url = raw_download_url(owner, repo_name, branch_ref, &entry.path);
            Some(Track {
                id: 0,
                repository_id,
                path: entry.path.clone(),
                name,
                format: ext,
                size: entry.size.unwrap_or(0),
                duration: None,
                url,
                local_path: None,
                downloaded: false,
                discovered_at: Utc::now(),
                favorite: false,
                blacklisted: false,
            })
        })
        .collect()
}

/// Raw-content URL for a path on a specific branch or sha.
pub fn raw_download_url(owner: &str, repo_name: &str, branch_ref: &str, path: &str) -> String {
    let encoded_path = path
        .split('/')
        .map(percent_encode_segment)
        .collect::<Vec<_>>()
        .join("/");
    format!("https://raw.githubusercontent.com/{owner}/{repo_name}/{branch_ref}/{encoded_path}")
}

fn percent_encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for ch in segment.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => {
                for byte in ch.to_string().as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

fn is_audio_format(ext: &str) -> bool {
    matches!(ext, "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "wma")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: &str, size: Option<u64>) -> TreeEntry {
        TreeEntry {
            path: path.to_string(),
            kind: kind.to_string(),
            size,
        }
    }

    #[test]
    fn keeps_only_audio_blobs_and_skips_directories() {
        let entries = vec![
            entry("README.md", "blob", Some(10)),
            entry("songs", "tree", None),
            entry("songs/track-1.mp3", "blob", Some(123)),
            entry("songs/track-2.flac", "blob", Some(456)),
            entry("docs/not-audio.txt", "blob", Some(7)),
        ];
        let tracks = tree_to_tracks(&entries, "alice", "music", "main", 42);
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].name, "track-1.mp3");
        assert_eq!(tracks[0].format, "mp3");
        assert_eq!(tracks[0].size, 123);
        assert_eq!(tracks[0].repository_id, 42);
        assert_eq!(tracks[1].name, "track-2.flac");
        assert_eq!(tracks[1].format, "flac");
    }

    #[test]
    fn builds_raw_content_url_with_branch_ref() {
        let entries = vec![entry("nested/dir/Song.MP3", "blob", Some(1))];
        let tracks = tree_to_tracks(&entries, "alice", "music", "main", 1);
        assert_eq!(tracks.len(), 1);
        assert_eq!(
            tracks[0].url,
            "https://raw.githubusercontent.com/alice/music/main/nested/dir/Song.MP3"
        );
    }

    #[test]
    fn percent_encodes_spaces_and_unicode_in_paths() {
        let url = raw_download_url("alice", "music", "main", "夜曲 (cover).mp3");
        // ASCII portion before the file: literal slashes preserved.
        assert!(url.starts_with("https://raw.githubusercontent.com/alice/music/main/"));
        // Space must be %20, parentheses left alone (unreserved-ish but acceptable),
        // and Unicode chars are percent-encoded.
        assert!(url.contains("%20"));
        assert!(!url.contains(' '));
        // The literal Unicode characters must not appear unencoded.
        assert!(!url.contains('夜'));
    }

    #[test]
    fn ignores_unknown_extensions_and_extensionless_blobs() {
        let entries = vec![
            entry("LICENSE", "blob", Some(10)),
            entry("songs/notes.txt", "blob", Some(10)),
            entry("songs/.gitkeep", "blob", Some(0)),
        ];
        let tracks = tree_to_tracks(&entries, "a", "b", "main", 1);
        assert!(tracks.is_empty());
    }

    #[test]
    fn case_insensitive_extension_matching() {
        let entries = vec![
            entry("a.MP3", "blob", Some(1)),
            entry("b.Flac", "blob", Some(1)),
            entry("c.WAV", "blob", Some(1)),
        ];
        let tracks = tree_to_tracks(&entries, "a", "b", "main", 1);
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].format, "mp3");
        assert_eq!(tracks[1].format, "flac");
        assert_eq!(tracks[2].format, "wav");
    }
}
