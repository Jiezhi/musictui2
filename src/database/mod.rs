use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection, Result};

use crate::models::{Repository, Track};

pub struct DatabaseManager {
    connection: Arc<Connection>,
}

impl DatabaseManager {
    pub fn new() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("musictui2");

        std::fs::create_dir_all(&config_dir).ok();

        let db_path = config_dir.join("music.db");
        let connection = Connection::open(db_path).expect("Failed to open database");

        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS repositories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                owner TEXT NOT NULL,
                name TEXT NOT NULL,
                url TEXT NOT NULL UNIQUE,
                added_at DATETIME NOT NULL,
                last_scanned DATETIME
            )",
                [],
            )
            .expect("Failed to create repositories table");

        connection
            .execute(
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
                FOREIGN KEY(repository_id) REFERENCES repositories(id)
            )",
                [],
            )
            .expect("Failed to create tracks table");

        Self {
            connection: Arc::new(connection),
        }
    }

    pub fn save_repository(&self, repository: &Repository) -> Result<()> {
        self.connection.execute(
            "INSERT INTO repositories (owner, name, url, added_at, last_scanned)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(url) DO UPDATE SET
               owner = excluded.owner,
               name = excluded.name,
               last_scanned = excluded.last_scanned",
            params![
                repository.owner,
                repository.name,
                repository.url,
                repository.added_at,
                repository.last_scanned,
            ],
        )?;

        Ok(())
    }

    pub fn get_repositories(&self) -> Result<Vec<Repository>> {
        let mut stmt = self.connection.prepare(
            "SELECT id, owner, name, url, added_at, last_scanned
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
        self.connection.execute(
            "DELETE FROM tracks WHERE repository_id = ?1",
            params![repository_id],
        )?;

        self.connection.execute(
            "DELETE FROM repositories WHERE id = ?1",
            params![repository_id],
        )?;

        Ok(())
    }

    pub fn delete_repository_by_name(&self, owner: &str, name: &str) -> Result<()> {
        let repository_id = self.connection.query_row(
            "SELECT id FROM repositories WHERE owner = ?1 AND name = ?2",
            params![owner, name],
            |row| row.get::<_, i64>(0),
        )?;

        self.delete_repository(repository_id)
    }

    pub fn save_track(&self, track: &Track) -> Result<()> {
        let duration_secs = track.duration.map(|d| d.as_secs() as i64);
        let local_path = track
            .local_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string());

        if track.id > 0 {
            self.connection.execute(
                "UPDATE tracks
                 SET repository_id = ?1,
                     path = ?2,
                     name = ?3,
                     format = ?4,
                     size = ?5,
                     duration = ?6,
                     url = ?7,
                     local_path = ?8,
                     downloaded = ?9
                 WHERE id = ?10",
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
                    track.id,
                ],
            )?;
        } else {
            self.connection.execute(
                "INSERT INTO tracks
                 (repository_id, path, name, format, size, duration, url, local_path, downloaded)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
                ],
            )?;
        }

        Ok(())
    }

    pub fn get_tracks_by_repo(&self, repository_id: i64) -> Result<Vec<Track>> {
        let mut stmt = self.connection.prepare(
            "SELECT id, repository_id, path, name, format, size, duration, url, local_path, downloaded
             FROM tracks
             WHERE repository_id = ?1
             ORDER BY name",
        )?;

        let tracks = stmt.query_map(params![repository_id], |row| {
            let duration_secs: Option<i64> = row.get(6)?;
            let local_path: Option<String> = row.get(8)?;

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
                discovered_at: row.get(10)?,
            })
        })?;

        tracks.collect()
    }

    pub fn get_repository_by_name(&self, owner: &str, name: &str) -> Result<Repository> {
        self.connection.query_row(
            "SELECT id, owner, name, url, added_at, last_scanned
             FROM repositories
             WHERE owner = ?1 AND name = ?2",
            params![owner, name],
            |row| {
                Ok(Repository {
                    id: row.get(0)?,
                    owner: row.get(1)?,
                    name: row.get(2)?,
                    url: row.get(3)?,
                    added_at: row.get(4)?,
                    last_scanned: row.get(5)?,
                    track_count: row.get(6)?,
                })
            },
        )
    }

    pub fn update_last_scanned(&self, owner: &str, name: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE repositories
             SET last_scanned = ?1
             WHERE owner = ?2 AND name = ?3",
            params![Utc::now(), owner, name],
        )?;

        Ok(())
    }

    pub fn get_track_by_id(&self, track_id: i64) -> Result<Track> {
        self.connection.query_row(
            "SELECT id, repository_id, path, name, format, size, duration, url, local_path, downloaded
             FROM tracks
             WHERE id = ?1",
            params![track_id],
            |row| {
                let duration_secs: Option<i64> = row.get(6)?;
                let local_path: Option<String> = row.get(8)?;

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
                    discovered_at: row.get(10)?,
                })
            },
        )
    }

    pub fn get_all_tracks(&self) -> Result<Vec<Track>> {
        let mut stmt = self.connection.prepare(
            "SELECT id, repository_id, path, name, format, size, duration, url, local_path, downloaded
             FROM tracks
             ORDER BY name",
        )?;

        let tracks = stmt.query_map([], |row| {
            let duration_secs: Option<i64> = row.get(6)?;
            let local_path: Option<String> = row.get(8)?;

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
                discovered_at: row.get(10)?,
            })
        })?;

        tracks.collect()
    }

    pub fn get_tracks_by_repo_name(&self, owner: &str, name: &str) -> Result<Vec<Track>> {
        // First get repository ID
        let repository_id = self.connection.query_row(
            "SELECT id FROM repositories WHERE owner = ?1 AND name = ?2",
            params![owner, name],
            |row| row.get::<_, i64>(0),
        )?;

        // Then get tracks for that repository
        self.get_tracks_by_repo(repository_id)
    }
}
