use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Repository {
    pub id: i64,
    pub owner: String,
    pub name: String,
    pub url: String,
    pub source_type: RepositorySource,
    pub cache_enabled: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    pub added_at: DateTime<Utc>,
    pub last_scanned: Option<DateTime<Utc>>,
    pub track_count: u32,
    /// Cached ETag of the last successful GitHub Git Trees response. Sent as
    /// `If-None-Match` on rescan so that an unchanged tree returns 304 and
    /// avoids re-walking the repository.
    #[serde(default)]
    pub tree_etag: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum RepositorySource {
    GitHub,
    WebDav,
}

impl RepositorySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::WebDav => "webdav",
        }
    }
}

impl std::str::FromStr for RepositorySource {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "github" => Ok(Self::GitHub),
            "webdav" => Ok(Self::WebDav),
            _ => Err(format!("Unknown repository source type: {value}")),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Track {
    pub id: i64,
    pub repository_id: i64,
    pub path: String,
    pub name: String,
    pub format: String,
    pub size: u64,
    pub duration: Option<std::time::Duration>,
    pub url: String,
    pub local_path: Option<PathBuf>,
    pub downloaded: bool,
    pub discovered_at: DateTime<Utc>,
    pub favorite: bool,
    pub blacklisted: bool,
}

impl Track {
    pub fn is_playable(&self) -> bool {
        self.downloaded && self.local_path.as_ref().is_some_and(|p| p.exists())
    }
}

#[derive(Debug, Clone)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone)]
pub struct AudioMetadata {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: std::time::Duration,
    pub format: String,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub cache_dir: PathBuf,
    pub max_cache_size: u64, // in bytes
    pub github_api_rate_limit: u32,
    pub audio_buffer_size: u32,
    pub theme: String,
}
