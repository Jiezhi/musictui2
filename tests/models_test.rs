use musictui2::models::{PlaybackState, Repository, Track};
use std::path::PathBuf;

#[test]
fn test_repository_creation() {
    let repo = Repository {
        id: 1,
        name: "test-repo".to_string(),
        owner: "test-owner".to_string(),
        url: "https://github.com/test-owner/test-repo".to_string(),
        added_at: chrono::Utc::now(),
        last_scanned: None,
        track_count: 0,
    };

    assert_eq!(repo.id, 1);
    assert_eq!(repo.name, "test-repo");
    assert_eq!(repo.owner, "test-owner");
    assert_eq!(repo.track_count, 0);
}

#[test]
fn test_track_creation() {
    let track = Track {
        id: 1,
        repository_id: 1,
        name: "test-track.mp3".to_string(),
        path: "/path/to/track.mp3".to_string(),
        url: "https://github.com/test-owner/test-repo/raw/main/track.mp3".to_string(),
        duration: Some(std::time::Duration::from_secs(180)),
        format: "mp3".to_string(),
        size: 1024 * 1024, // 1MB
        downloaded: false,
        local_path: None,
        discovered_at: chrono::Utc::now(),
    };

    assert_eq!(track.id, 1);
    assert_eq!(track.name, "test-track.mp3");
    assert_eq!(track.downloaded, false);
    assert!(track.local_path.is_none());
}

#[test]
fn test_playback_states() {
    assert!(matches!(PlaybackState::Playing, PlaybackState::Playing));
    assert!(matches!(PlaybackState::Paused, PlaybackState::Paused));
    assert!(matches!(PlaybackState::Stopped, PlaybackState::Stopped));
}

#[test]
fn test_track_is_playable() {
    use std::io::Write;
    use tempfile::NamedTempFile;

    // Create a temporary file for testing
    let mut temp_file = NamedTempFile::new().unwrap();
    write!(temp_file, "test audio content").unwrap();
    let temp_path = temp_file.path().to_path_buf();

    // Track with no local path - not playable
    let track1 = Track {
        id: 1,
        repository_id: 1,
        name: "test-track.mp3".to_string(),
        path: "/path/to/track.mp3".to_string(),
        url: "https://github.com/test-owner/test-repo/raw/main/track.mp3".to_string(),
        duration: Some(std::time::Duration::from_secs(180)),
        format: "mp3".to_string(),
        size: 1024 * 1024,
        downloaded: false,
        local_path: None,
        discovered_at: chrono::Utc::now(),
    };
    assert!(!track1.is_playable());

    // Track with local path to existing file and downloaded - playable
    let track2 = Track {
        id: 2,
        repository_id: 1,
        name: "test-track-existing.mp3".to_string(),
        path: "/path/to/track.mp3".to_string(),
        url: "https://github.com/test-owner/test-repo/raw/main/track.mp3".to_string(),
        duration: Some(std::time::Duration::from_secs(180)),
        format: "mp3".to_string(),
        size: 1024 * 1024,
        downloaded: true,
        local_path: Some(temp_path.clone()),
        discovered_at: chrono::Utc::now(),
    };
    assert!(track2.is_playable());

    // Track with local path to non-existent file - not playable
    let track3 = Track {
        id: 3,
        repository_id: 1,
        name: "test-track-nonexistent.mp3".to_string(),
        path: "/path/to/track.mp3".to_string(),
        url: "https://github.com/test-owner/test-repo/raw/main/track.mp3".to_string(),
        duration: Some(std::time::Duration::from_secs(180)),
        format: "mp3".to_string(),
        size: 1024 * 1024,
        downloaded: true,
        local_path: Some(PathBuf::from("/non/existent/file.mp3")),
        discovered_at: chrono::Utc::now(),
    };
    assert!(!track3.is_playable());

    // Track with downloaded but no local path - not playable
    let track4 = Track {
        id: 4,
        repository_id: 1,
        name: "test-track-no-path.mp3".to_string(),
        path: "/path/to/track.mp3".to_string(),
        url: "https://github.com/test-owner/test-repo/raw/main/track.mp3".to_string(),
        duration: Some(std::time::Duration::from_secs(180)),
        format: "mp3".to_string(),
        size: 1024 * 1024,
        downloaded: true,
        local_path: None,
        discovered_at: chrono::Utc::now(),
    };
    assert!(!track4.is_playable());

    // Cleanup
    drop(temp_file);
}
