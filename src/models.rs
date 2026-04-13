use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use chrono::{DateTime, Utc};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Repository {
    pub id: i64,
    pub owner: String,
    pub name: String,
    pub url: String,
    pub added_at: DateTime<Utc>,
    pub last_scanned: Option<DateTime<Utc>>,
    pub track_count: u32,
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
}

impl Track {
    pub fn is_playable(&self) -> bool {
        self.downloaded && self.local_path.as_ref().map_or(false, |p| p.exists())
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