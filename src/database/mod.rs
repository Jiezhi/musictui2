use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::{params, Connection, Result};

use crate::models::{Repository, Track};

pub struct DatabaseManager {
    connection: Arc<Mutex<Connection>>,
}

impl DatabaseManager {
    pub fn new() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("musictui2");

        std::fs::create_dir_all(&config_dir).ok();

        let db_path = config_dir.join("music.db");
        Self::from_path(db_path).expect("Failed to open database")
    }

    #[allow(dead_code)]
    pub fn from_path(db_path: impl AsRef<std::path::Path>) -> Result<Self> {
        let connection = Connection::open(db_path)?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS repositories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                owner TEXT NOT NULL,
                name TEXT NOT NULL,
                url TEXT NOT NULL UNIQUE,
                added_at DATETIME NOT NULL,
                last_scanned DATETIME,
                track_count INTEGER DEFAULT 0
            )",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS tracks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repository_id INTEGER NOT NULL,
                path TEXT NOT NULL,
                name TEXT NOT NULL,
                format TEXT NOT NULL,
                size INTEGER NOT NULL,
                duration INTEGER,
                url TEXT NOT NULL,
                local_path TEXT,
                downloaded BOOLEAN DEFAULT FALSE,
                discovered_at DATETIME,
                FOREIGN KEY(repository_id) REFERENCES repositories(id)
            )",
            [],
        )?;

        let db = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        db.ensure_schema()?;
        Ok(db)
    }

    fn ensure_schema(&self) -> Result<()> {
        let conn = self.connection.lock().unwrap();

        if !Self::has_column(&conn, "repositories", "track_count")? {
            conn.execute(
                "ALTER TABLE repositories ADD COLUMN track_count INTEGER DEFAULT 0",
                [],
            )?;
        }

        if !Self::has_column(&conn, "tracks", "discovered_at")? {
            conn.execute("ALTER TABLE tracks ADD COLUMN discovered_at DATETIME", [])?;
        }

        let missing_discovered_at: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tracks WHERE discovered_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        if missing_discovered_at > 0 {
            conn.execute(
                "UPDATE tracks SET discovered_at = ?1 WHERE discovered_at IS NULL",
                params![Utc::now()],
            )?;
        }

        Self::remove_duplicate_tracks(&conn)?;
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_tracks_repository_path
             ON tracks(repository_id, path)",
            [],
        )?;

        Ok(())
    }

    fn remove_duplicate_tracks(conn: &Connection) -> Result<()> {
        conn.execute(
            "DELETE FROM tracks
             WHERE id NOT IN (
                SELECT
                    CASE
                        WHEN MAX(CASE WHEN downloaded THEN id ELSE NULL END) IS NOT NULL
                        THEN MAX(CASE WHEN downloaded THEN id ELSE NULL END)
                        ELSE MIN(id)
                    END
                FROM tracks
                GROUP BY repository_id, path
             )",
            [],
        )?;

        Ok(())
    }

    fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;

        for name in columns {
            if name? == column {
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub fn save_repository(&self, repository: &Repository) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT INTO repositories (owner, name, url, added_at, last_scanned, track_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(url) DO UPDATE SET
                owner = excluded.owner,
                name = excluded.name",
            params![
                repository.owner,
                repository.name,
                repository.url,
                repository.added_at,
                repository.last_scanned,
                repository.track_count,
            ],
        )?;

        Ok(())
    }

    pub fn get_repositories(&self) -> Result<Vec<Repository>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, owner, name, url, added_at, last_scanned, track_count
             FROM repositories
             ORDER BY added_at DESC",
        )?;

        let repositories = stmt.query_map([], |row| {
            Ok(Repository {
                id: row.get(0)?,
                owner: row.get(1)?,
                name: row.get(2)?,
                url: row.get(3)?,
                added_at: row.get(4)?,
                last_scanned: row.get(5)?,
                track_count: row.get(6)?,
            })
        })?;

        repositories.collect()
    }

    pub fn delete_repository(&self, repository_id: i64) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "DELETE FROM tracks WHERE repository_id = ?1",
            params![repository_id],
        )?;
        conn.execute(
            "DELETE FROM repositories WHERE id = ?1",
            params![repository_id],
        )?;

        Ok(())
    }

    pub fn save_track(&self, track: &Track) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        let duration_secs = track.duration.map(|d| d.as_secs() as i64);
        let local_path = track
            .local_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());

        conn.execute(
            "INSERT INTO tracks (
                repository_id, path, name, format, size, duration, url, local_path, downloaded, discovered_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(repository_id, path) DO UPDATE SET
                name = excluded.name,
                format = excluded.format,
                size = excluded.size,
                duration = excluded.duration,
                url = excluded.url,
                local_path = COALESCE(excluded.local_path, tracks.local_path),
                downloaded = tracks.downloaded OR excluded.downloaded,
                discovered_at = tracks.discovered_at",
            params![
                track.repository_id,
                track.path,
                track.name,
                track.format,
                track.size as i64,
                duration_secs,
                track.url,
                local_path,
                track.downloaded,
                track.discovered_at,
            ],
        )?;

        Ok(())
    }

    pub fn get_tracks_by_repo(&self, repository_id: i64) -> Result<Vec<Track>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repository_id, path, name, format, size, duration, url, local_path, downloaded, discovered_at
             FROM tracks
             WHERE repository_id = ?1
             ORDER BY name",
        )?;

        let tracks = stmt.query_map(params![repository_id], |row| {
            let duration_secs: Option<i64> = row.get(6)?;
            let local_path: Option<String> = row.get(8)?;
            let discovered_at: chrono::DateTime<chrono::Utc> = row.get(10)?;

            Ok(Track {
                id: row.get(0)?,
                repository_id: row.get(1)?,
                path: row.get(2)?,
                name: row.get(3)?,
                format: row.get(4)?,
                size: row.get::<_, i64>(5)? as u64,
                duration: duration_secs.map(|secs| std::time::Duration::from_secs(secs as u64)),
                url: row.get(7)?,
                local_path: local_path.map(Into::into),
                downloaded: row.get(9)?,
                discovered_at,
            })
        })?;

        tracks.collect()
    }

    pub fn get_tracks_by_repo_name(&self, owner: &str, name: &str) -> Result<Vec<Track>> {
        let repository_id = {
            let conn = self.connection.lock().unwrap();
            conn.query_row(
                "SELECT id FROM repositories WHERE owner = ?1 AND name = ?2",
                params![owner, name],
                |row| row.get::<_, i64>(0),
            )?
        };

        self.get_tracks_by_repo(repository_id)
    }

    pub fn update_last_scanned(&self, owner: &str, name: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE repositories
             SET last_scanned = ?1
             WHERE owner = ?2 AND name = ?3",
            params![Utc::now(), owner, name],
        )?;

        Ok(())
    }

    pub fn get_repository_by_name(&self, owner: &str, name: &str) -> Result<Repository> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, owner, name, url, added_at, last_scanned, track_count
             FROM repositories
             WHERE owner = ?1 AND name = ?2",
        )?;

        let row = stmt.query_row(params![owner, name], |row| {
            Ok(Repository {
                id: row.get(0)?,
                owner: row.get(1)?,
                name: row.get(2)?,
                url: row.get(3)?,
                added_at: row.get(4)?,
                last_scanned: row.get(5)?,
                track_count: row.get(6)?,
            })
        })?;

        Ok(row)
    }

    pub fn get_track_by_id(&self, track_id: i64) -> Result<Track> {
        let conn = self.connection.lock().unwrap();
        conn.query_row(
            "SELECT id, repository_id, path, name, format, size, duration, url, local_path, downloaded, discovered_at
             FROM tracks
             WHERE id = ?1",
            params![track_id],
            |row| {
                let duration_secs: Option<i64> = row.get(6)?;
                let local_path: Option<String> = row.get(8)?;
                let discovered_at: chrono::DateTime<chrono::Utc> = row.get(10)?;

                Ok(Track {
                    id: row.get(0)?,
                    repository_id: row.get(1)?,
                    path: row.get(2)?,
                    name: row.get(3)?,
                    format: row.get(4)?,
                    size: row.get::<_, i64>(5)? as u64,
                    duration: duration_secs.map(|secs| std::time::Duration::from_secs(secs as u64)),
                    url: row.get(7)?,
                    local_path: local_path.map(Into::into),
                    downloaded: row.get(9)?,
                    discovered_at,
                })
            },
        )
    }

    pub fn get_all_tracks(&self) -> Result<Vec<Track>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repository_id, path, name, format, size, duration, url, local_path, downloaded, discovered_at
             FROM tracks
             ORDER BY name",
        )?;

        let tracks = stmt.query_map([], |row| {
            let duration_secs: Option<i64> = row.get(6)?;
            let local_path: Option<String> = row.get(8)?;
            let discovered_at: chrono::DateTime<chrono::Utc> = row.get(10)?;

            Ok(Track {
                id: row.get(0)?,
                repository_id: row.get(1)?,
                path: row.get(2)?,
                name: row.get(3)?,
                format: row.get(4)?,
                size: row.get::<_, i64>(5)? as u64,
                duration: duration_secs.map(|secs| std::time::Duration::from_secs(secs as u64)),
                url: row.get(7)?,
                local_path: local_path.map(Into::into),
                downloaded: row.get(9)?,
                discovered_at,
            })
        })?;

        tracks.collect()
    }
}

unsafe impl Sync for DatabaseManager {}
unsafe impl Send for DatabaseManager {}

impl Default for DatabaseManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn migrates_legacy_tracks_without_discovered_at() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        let conn = Connection::open(&db_path).unwrap();

        conn.execute(
            "CREATE TABLE repositories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                owner TEXT NOT NULL,
                name TEXT NOT NULL,
                url TEXT NOT NULL UNIQUE,
                added_at DATETIME NOT NULL,
                last_scanned DATETIME
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE tracks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repository_id INTEGER NOT NULL,
                path TEXT NOT NULL,
                name TEXT NOT NULL,
                format TEXT NOT NULL,
                size INTEGER NOT NULL,
                duration INTEGER,
                url TEXT NOT NULL,
                local_path TEXT,
                downloaded BOOLEAN DEFAULT FALSE,
                FOREIGN KEY(repository_id) REFERENCES repositories(id)
            )",
            [],
        )
        .unwrap();

        let added_at = Utc::now();
        conn.execute(
            "INSERT INTO repositories (owner, name, url, added_at, last_scanned)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "owner",
                "repo",
                "https://github.com/owner/repo",
                added_at,
                Option::<chrono::DateTime<Utc>>::None,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracks (
                repository_id, path, name, format, size, duration, url, local_path, downloaded
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                1_i64,
                "song.mp3",
                "song.mp3",
                "mp3",
                1024_i64,
                Option::<i64>::None,
                "https://example.com/song.mp3",
                Option::<String>::None,
                false,
            ],
        )
        .unwrap();
        drop(conn);

        let db = DatabaseManager::from_path(&db_path).unwrap();
        let repos = db.get_repositories().unwrap();
        let tracks = db.get_tracks_by_repo(repos[0].id).unwrap();

        assert_eq!(repos[0].track_count, 0);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].name, "song.mp3");
    }

    #[test]
    fn saving_existing_repository_keeps_tracks() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        let db = DatabaseManager::from_path(&db_path).unwrap();

        let repository = Repository {
            id: 0,
            owner: "owner".to_string(),
            name: "repo".to_string(),
            url: "https://github.com/owner/repo".to_string(),
            added_at: Utc::now(),
            last_scanned: None,
            track_count: 1,
        };

        db.save_repository(&repository).unwrap();
        let saved_repository = db.get_repository_by_name("owner", "repo").unwrap();

        let track = Track {
            id: 0,
            repository_id: saved_repository.id,
            path: "song.mp3".to_string(),
            name: "song.mp3".to_string(),
            format: "mp3".to_string(),
            size: 1024,
            duration: None,
            url: "https://example.com/song.mp3".to_string(),
            local_path: None,
            downloaded: false,
            discovered_at: Utc::now(),
        };
        db.save_track(&track).unwrap();

        db.save_repository(&repository).unwrap();

        let tracks = db.get_tracks_by_repo(saved_repository.id).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].repository_id, saved_repository.id);
    }

    #[test]
    fn saving_existing_track_updates_without_duplicates() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        let db = DatabaseManager::from_path(&db_path).unwrap();

        let repository = Repository {
            id: 0,
            owner: "owner".to_string(),
            name: "repo".to_string(),
            url: "https://github.com/owner/repo".to_string(),
            added_at: Utc::now(),
            last_scanned: None,
            track_count: 0,
        };

        db.save_repository(&repository).unwrap();
        let saved_repository = db.get_repository_by_name("owner", "repo").unwrap();

        let mut track = Track {
            id: 0,
            repository_id: saved_repository.id,
            path: "song.mp3".to_string(),
            name: "song.mp3".to_string(),
            format: "mp3".to_string(),
            size: 1024,
            duration: None,
            url: "https://example.com/song.mp3".to_string(),
            local_path: None,
            downloaded: false,
            discovered_at: Utc::now(),
        };

        db.save_track(&track).unwrap();
        track.size = 2048;
        db.save_track(&track).unwrap();

        let tracks = db.get_tracks_by_repo(saved_repository.id).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].size, 2048);
    }

    #[test]
    fn deleting_repository_removes_tracks() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        let db = DatabaseManager::from_path(&db_path).unwrap();

        let repository = Repository {
            id: 0,
            owner: "owner".to_string(),
            name: "repo".to_string(),
            url: "https://github.com/owner/repo".to_string(),
            added_at: Utc::now(),
            last_scanned: None,
            track_count: 0,
        };

        db.save_repository(&repository).unwrap();
        let saved_repository = db.get_repository_by_name("owner", "repo").unwrap();

        let track = Track {
            id: 0,
            repository_id: saved_repository.id,
            path: "song.mp3".to_string(),
            name: "song.mp3".to_string(),
            format: "mp3".to_string(),
            size: 1024,
            duration: None,
            url: "https://example.com/song.mp3".to_string(),
            local_path: None,
            downloaded: false,
            discovered_at: Utc::now(),
        };

        db.save_track(&track).unwrap();
        db.delete_repository(saved_repository.id).unwrap();

        assert!(db
            .get_tracks_by_repo(saved_repository.id)
            .unwrap()
            .is_empty());
        assert!(db.get_repository_by_name("owner", "repo").is_err());
    }
}
