use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::{params, Connection, Result, Row};

use crate::models::{Repository, Track};

const REPOSITORY_COLUMNS: &str =
    "id, owner, name, url, source_type, cache_enabled, username, password, added_at, last_scanned, track_count, tree_etag";

const TRACK_COLUMNS: &str = "id, repository_id, path, name, format, size, duration, url, local_path, downloaded, discovered_at, favorite, blacklisted";

fn row_to_repository(row: &Row<'_>) -> Result<Repository> {
    let source_type: String = row.get(4)?;
    Ok(Repository {
        id: row.get(0)?,
        owner: row.get(1)?,
        name: row.get(2)?,
        url: row.get(3)?,
        source_type: source_type
            .parse()
            .map_err(rusqlite::Error::InvalidParameterName)?,
        cache_enabled: row.get(5)?,
        username: row.get(6)?,
        password: row.get(7)?,
        added_at: row.get(8)?,
        last_scanned: row.get(9)?,
        track_count: row.get(10)?,
        tree_etag: row.get(11)?,
    })
}

fn row_to_track(row: &Row<'_>) -> Result<Track> {
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
        favorite: row.get(11)?,
        blacklisted: row.get(12)?,
    })
}

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

        // Enable WAL mode for better concurrent read/write performance.
        // WAL allows readers to continue during writes — important for the TUI
        // browsing while a scan is in-flight.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        // synchronous=NORMAL is safe with WAL and gives a substantial speedup;
        // FULL is overkill for a local user library, OFF is dangerous.
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        // SQLite ignores FOREIGN KEY constraints unless explicitly enabled.
        connection.pragma_update(None, "foreign_keys", "ON")?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS repositories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                owner TEXT NOT NULL,
                name TEXT NOT NULL,
                url TEXT NOT NULL UNIQUE,
                source_type TEXT NOT NULL DEFAULT 'github',
                cache_enabled BOOLEAN DEFAULT TRUE,
                username TEXT,
                password TEXT,
                added_at DATETIME NOT NULL,
                last_scanned DATETIME,
                track_count INTEGER DEFAULT 0,
                tree_etag TEXT
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
                favorite BOOLEAN DEFAULT FALSE,
                blacklisted BOOLEAN DEFAULT FALSE,
                FOREIGN KEY(repository_id) REFERENCES repositories(id) ON DELETE CASCADE
            )",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
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

        if !Self::has_column(&conn, "repositories", "source_type")? {
            conn.execute(
                "ALTER TABLE repositories ADD COLUMN source_type TEXT NOT NULL DEFAULT 'github'",
                [],
            )?;
        }

        if !Self::has_column(&conn, "repositories", "cache_enabled")? {
            conn.execute(
                "ALTER TABLE repositories ADD COLUMN cache_enabled BOOLEAN DEFAULT TRUE",
                [],
            )?;
        }

        if !Self::has_column(&conn, "repositories", "username")? {
            conn.execute("ALTER TABLE repositories ADD COLUMN username TEXT", [])?;
        }

        if !Self::has_column(&conn, "repositories", "password")? {
            conn.execute("ALTER TABLE repositories ADD COLUMN password TEXT", [])?;
        }

        if !Self::has_column(&conn, "repositories", "tree_etag")? {
            conn.execute("ALTER TABLE repositories ADD COLUMN tree_etag TEXT", [])?;
        }

        if !Self::has_column(&conn, "tracks", "discovered_at")? {
            conn.execute("ALTER TABLE tracks ADD COLUMN discovered_at DATETIME", [])?;
        }

        if !Self::has_column(&conn, "tracks", "favorite")? {
            conn.execute(
                "ALTER TABLE tracks ADD COLUMN favorite BOOLEAN DEFAULT FALSE",
                [],
            )?;
        }

        if !Self::has_column(&conn, "tracks", "blacklisted")? {
            conn.execute(
                "ALTER TABLE tracks ADD COLUMN blacklisted BOOLEAN DEFAULT FALSE",
                [],
            )?;
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
        // Partial indexes for the favorite/blacklist filter pushdown — only
        // tracks matching the predicate occupy the index, keeping it tiny.
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tracks_favorite
             ON tracks(name) WHERE favorite = 1 AND blacklisted = 0",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_tracks_blacklisted
             ON tracks(name) WHERE blacklisted = 1",
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
            "INSERT INTO repositories (owner, name, url, source_type, cache_enabled, username, password, added_at, last_scanned, track_count, tree_etag)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(url) DO UPDATE SET
                owner = excluded.owner,
                name = excluded.name,
                source_type = excluded.source_type,
                cache_enabled = excluded.cache_enabled,
                username = excluded.username,
                password = excluded.password,
                tree_etag = excluded.tree_etag",
            params![
                repository.owner,
                repository.name,
                repository.url,
                repository.source_type.as_str(),
                repository.cache_enabled,
                repository.username,
                repository.password,
                repository.added_at,
                repository.last_scanned,
                repository.track_count,
                repository.tree_etag,
            ],
        )?;

        Ok(())
    }

    pub fn get_repositories(&self) -> Result<Vec<Repository>> {
        let conn = self.connection.lock().unwrap();
        let sql = format!("SELECT {REPOSITORY_COLUMNS} FROM repositories ORDER BY added_at DESC");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_repository)?;
        rows.collect()
    }

    pub fn delete_repository(&self, repository_id: i64) -> Result<()> {
        let mut conn = self.connection.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM tracks WHERE repository_id = ?1",
            params![repository_id],
        )?;
        tx.execute(
            "DELETE FROM repositories WHERE id = ?1",
            params![repository_id],
        )?;
        tx.commit()?;

        Ok(())
    }

    pub fn save_track(&self, track: &Track) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        Self::execute_save_track(&conn, track)
    }

    /// Save many tracks in a single transaction. Used by scanners to avoid one
    /// fsync per track when ingesting large repositories.
    pub fn save_tracks(&self, tracks: &[Track]) -> Result<()> {
        if tracks.is_empty() {
            return Ok(());
        }

        let mut conn = self.connection.lock().unwrap();
        let tx = conn.transaction()?;
        for track in tracks {
            Self::execute_save_track(&tx, track)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn execute_save_track(
        conn: &impl std::ops::Deref<Target = Connection>,
        track: &Track,
    ) -> Result<()> {
        let duration_secs = track.duration.map(|d| d.as_secs() as i64);
        let local_path = track
            .local_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());

        conn.execute(
            "INSERT INTO tracks (
                repository_id, path, name, format, size, duration, url, local_path, downloaded, discovered_at, favorite, blacklisted
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(repository_id, path) DO UPDATE SET
                name = excluded.name,
                format = excluded.format,
                size = excluded.size,
                duration = excluded.duration,
                url = excluded.url,
                local_path = COALESCE(excluded.local_path, tracks.local_path),
                downloaded = tracks.downloaded OR excluded.downloaded,
                discovered_at = tracks.discovered_at,
                favorite = tracks.favorite OR excluded.favorite,
                blacklisted = tracks.blacklisted OR excluded.blacklisted",
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
                track.favorite,
                track.blacklisted,
            ],
        )?;

        Ok(())
    }

    pub fn get_tracks_by_repo(&self, repository_id: i64) -> Result<Vec<Track>> {
        let conn = self.connection.lock().unwrap();
        let sql =
            format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE repository_id = ?1 ORDER BY name");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![repository_id], row_to_track)?;
        rows.collect()
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

    pub fn update_last_scanned_by_id(&self, repository_id: i64) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE repositories
             SET last_scanned = ?1
             WHERE id = ?2",
            params![Utc::now(), repository_id],
        )?;

        Ok(())
    }

    /// Persists a freshly-observed ETag for the repository's git tree so that
    /// subsequent scans can send `If-None-Match` and short-circuit on a 304.
    pub fn update_tree_etag(&self, repository_id: i64, etag: Option<&str>) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE repositories SET tree_etag = ?1 WHERE id = ?2",
            params![etag, repository_id],
        )?;

        Ok(())
    }

    pub fn get_repository_by_name(&self, owner: &str, name: &str) -> Result<Repository> {
        let conn = self.connection.lock().unwrap();
        let sql =
            format!("SELECT {REPOSITORY_COLUMNS} FROM repositories WHERE owner = ?1 AND name = ?2");
        conn.query_row(&sql, params![owner, name], row_to_repository)
    }

    pub fn get_repository_by_id(&self, repository_id: i64) -> Result<Repository> {
        let conn = self.connection.lock().unwrap();
        let sql = format!("SELECT {REPOSITORY_COLUMNS} FROM repositories WHERE id = ?1");
        conn.query_row(&sql, params![repository_id], row_to_repository)
    }

    pub fn get_track_by_id(&self, track_id: i64) -> Result<Track> {
        let conn = self.connection.lock().unwrap();
        let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id = ?1");
        conn.query_row(&sql, params![track_id], row_to_track)
    }

    pub fn get_all_tracks(&self) -> Result<Vec<Track>> {
        let conn = self.connection.lock().unwrap();
        let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks ORDER BY name");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_track)?;
        rows.collect()
    }

    pub fn set_track_favorite(&self, track_id: i64, favorite: bool) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE tracks SET favorite = ?1 WHERE id = ?2",
            params![favorite, track_id],
        )?;

        Ok(())
    }

    pub fn set_track_blacklisted(&self, track_id: i64, blacklisted: bool) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "UPDATE tracks
             SET blacklisted = ?1,
                 favorite = CASE WHEN ?1 THEN FALSE ELSE favorite END
             WHERE id = ?2",
            params![blacklisted, track_id],
        )?;

        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;

        Ok(())
    }

    pub fn get_favorite_tracks(&self) -> Result<Vec<Track>> {
        let conn = self.connection.lock().unwrap();
        let sql = format!(
            "SELECT {TRACK_COLUMNS} FROM tracks WHERE favorite = 1 AND blacklisted = 0 ORDER BY name"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_track)?;
        rows.collect()
    }

    pub fn get_blacklisted_tracks(&self) -> Result<Vec<Track>> {
        let conn = self.connection.lock().unwrap();
        let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE blacklisted = 1 ORDER BY name");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_track)?;
        rows.collect()
    }
}

impl Default for DatabaseManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_repository() -> Repository {
        Repository {
            id: 0,
            owner: "owner".to_string(),
            name: "repo".to_string(),
            url: "https://github.com/owner/repo".to_string(),
            source_type: crate::models::RepositorySource::GitHub,
            cache_enabled: true,
            username: None,
            password: None,
            added_at: Utc::now(),
            last_scanned: None,
            track_count: 0,
            tree_etag: None,
        }
    }

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
        assert!(!tracks[0].favorite);
        assert!(!tracks[0].blacklisted);
    }

    #[test]
    fn saving_existing_repository_keeps_tracks() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        let db = DatabaseManager::from_path(&db_path).unwrap();

        let mut repository = test_repository();
        repository.track_count = 1;

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
            favorite: false,
            blacklisted: false,
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

        let repository = test_repository();

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
            favorite: false,
            blacklisted: false,
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

        let repository = test_repository();

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
            favorite: false,
            blacklisted: false,
        };

        db.save_track(&track).unwrap();
        db.delete_repository(saved_repository.id).unwrap();

        assert!(db
            .get_tracks_by_repo(saved_repository.id)
            .unwrap()
            .is_empty());
        assert!(db.get_repository_by_name("owner", "repo").is_err());
    }

    #[test]
    fn track_favorite_and_blacklist_flags_are_persisted() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        let db = DatabaseManager::from_path(&db_path).unwrap();

        let repository = test_repository();

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
            favorite: true,
            blacklisted: false,
        };

        db.save_track(&track).unwrap();
        let saved_track = db.get_tracks_by_repo(saved_repository.id).unwrap()[0].clone();
        assert!(saved_track.favorite);
        assert!(!saved_track.blacklisted);

        db.set_track_blacklisted(saved_track.id, true).unwrap();
        let blacklisted_track = db.get_track_by_id(saved_track.id).unwrap();
        assert!(!blacklisted_track.favorite);
        assert!(blacklisted_track.blacklisted);
        assert!(db.get_favorite_tracks().unwrap().is_empty());
        assert_eq!(db.get_blacklisted_tracks().unwrap().len(), 1);

        track.size = 2048;
        track.favorite = false;
        track.blacklisted = false;
        db.save_track(&track).unwrap();

        let rescanned_track = db.get_track_by_id(saved_track.id).unwrap();
        assert!(rescanned_track.blacklisted);
    }

    fn test_track(repository_id: i64, path: &str) -> Track {
        Track {
            id: 0,
            repository_id,
            path: path.to_string(),
            name: path.to_string(),
            format: "mp3".to_string(),
            size: 1024,
            duration: None,
            url: format!("https://example.com/{path}"),
            local_path: None,
            downloaded: false,
            discovered_at: Utc::now(),
            favorite: false,
            blacklisted: false,
        }
    }

    #[test]
    fn pragmas_are_applied_on_open() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        let db = DatabaseManager::from_path(&db_path).unwrap();
        let conn = db.connection.lock().unwrap();

        let journal: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");

        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn save_tracks_commits_atomically() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        let db = DatabaseManager::from_path(&db_path).unwrap();

        db.save_repository(&test_repository()).unwrap();
        let repo = db.get_repository_by_name("owner", "repo").unwrap();

        let tracks: Vec<Track> = (0..50)
            .map(|i| test_track(repo.id, &format!("track-{i}.mp3")))
            .collect();
        db.save_tracks(&tracks).unwrap();

        let persisted = db.get_tracks_by_repo(repo.id).unwrap();
        assert_eq!(persisted.len(), 50);
    }

    #[test]
    fn save_tracks_is_a_no_op_for_empty_input() {
        let dir = tempdir().unwrap();
        let db = DatabaseManager::from_path(dir.path().join("music.db")).unwrap();
        // Should not panic, should not need a valid repository row.
        db.save_tracks(&[]).unwrap();
    }

    #[test]
    fn save_tracks_rolls_back_on_failure() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        let db = DatabaseManager::from_path(&db_path).unwrap();

        db.save_repository(&test_repository()).unwrap();
        let repo = db.get_repository_by_name("owner", "repo").unwrap();

        // First track has a valid repo, second references a nonexistent repo.
        // With foreign_keys = ON, the second insert must fail and the entire
        // batch must roll back so the first track is not persisted either.
        let tracks = vec![test_track(repo.id, "ok.mp3"), test_track(9999, "bad.mp3")];

        let result = db.save_tracks(&tracks);
        assert!(result.is_err(), "expected FK violation");

        let persisted = db.get_tracks_by_repo(repo.id).unwrap();
        assert!(
            persisted.is_empty(),
            "transaction should have rolled back, found {} tracks",
            persisted.len()
        );
    }

    #[test]
    fn delete_repository_removes_tracks_via_transaction() {
        let dir = tempdir().unwrap();
        let db = DatabaseManager::from_path(dir.path().join("music.db")).unwrap();

        db.save_repository(&test_repository()).unwrap();
        let repo = db.get_repository_by_name("owner", "repo").unwrap();
        db.save_tracks(&[test_track(repo.id, "a.mp3"), test_track(repo.id, "b.mp3")])
            .unwrap();

        db.delete_repository(repo.id).unwrap();

        assert!(db.get_tracks_by_repo(repo.id).unwrap().is_empty());
        assert!(matches!(
            db.get_repository_by_id(repo.id),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
    }

    #[test]
    fn favorite_filter_is_applied_in_sql_and_excludes_blacklisted() {
        let dir = tempdir().unwrap();
        let db = DatabaseManager::from_path(dir.path().join("music.db")).unwrap();

        db.save_repository(&test_repository()).unwrap();
        let repo = db.get_repository_by_name("owner", "repo").unwrap();

        let mut fav = test_track(repo.id, "fav.mp3");
        fav.favorite = true;
        let mut both = test_track(repo.id, "both.mp3");
        both.favorite = true;
        both.blacklisted = true;
        let plain = test_track(repo.id, "plain.mp3");

        db.save_tracks(&[fav, both, plain]).unwrap();

        let favorites = db.get_favorite_tracks().unwrap();
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].name, "fav.mp3");

        let blacklisted = db.get_blacklisted_tracks().unwrap();
        assert_eq!(blacklisted.len(), 1);
        assert_eq!(blacklisted[0].name, "both.mp3");
    }

    #[test]
    fn favorite_index_is_used_by_query_planner() {
        // Without the partial index, SQLite scans the whole tracks table.
        // With it, the query plan must mention the index. We use EXPLAIN
        // QUERY PLAN to verify.
        let dir = tempdir().unwrap();
        let db = DatabaseManager::from_path(dir.path().join("music.db")).unwrap();
        let conn = db.connection.lock().unwrap();

        let sql = format!(
            "EXPLAIN QUERY PLAN SELECT {TRACK_COLUMNS} FROM tracks WHERE favorite = 1 AND blacklisted = 0 ORDER BY name"
        );
        let plan: Vec<String> = conn
            .prepare(&sql)
            .unwrap()
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        let joined = plan.join("\n");
        assert!(
            joined.contains("idx_tracks_favorite"),
            "expected query plan to use idx_tracks_favorite, got:\n{joined}"
        );
    }

    #[test]
    fn database_manager_is_send_and_sync_without_unsafe_impls() {
        // Arc<Mutex<Connection>> is Send + Sync by construction, so DatabaseManager
        // does not need any `unsafe impl`. This test fails to compile if that is
        // ever no longer the case.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DatabaseManager>();
    }

    #[test]
    fn tree_etag_round_trips_through_save_and_update() {
        let dir = tempdir().unwrap();
        let db = DatabaseManager::from_path(dir.path().join("music.db")).unwrap();

        let mut repository = test_repository();
        repository.tree_etag = Some("W/\"abc123\"".to_string());
        db.save_repository(&repository).unwrap();
        let stored = db.get_repository_by_name("owner", "repo").unwrap();
        assert_eq!(stored.tree_etag.as_deref(), Some("W/\"abc123\""));

        db.update_tree_etag(stored.id, Some("W/\"def456\""))
            .unwrap();
        let refreshed = db.get_repository_by_name("owner", "repo").unwrap();
        assert_eq!(refreshed.tree_etag.as_deref(), Some("W/\"def456\""));

        db.update_tree_etag(stored.id, None).unwrap();
        let cleared = db.get_repository_by_name("owner", "repo").unwrap();
        assert_eq!(cleared.tree_etag, None);
    }

    #[test]
    fn migrates_legacy_repositories_to_add_tree_etag_column() {
        // Simulate a pre-Phase-2 database that lacks the tree_etag column.
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute(
                "CREATE TABLE repositories (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    owner TEXT NOT NULL,
                    name TEXT NOT NULL,
                    url TEXT NOT NULL UNIQUE,
                    source_type TEXT NOT NULL DEFAULT 'github',
                    cache_enabled BOOLEAN DEFAULT TRUE,
                    username TEXT,
                    password TEXT,
                    added_at DATETIME NOT NULL,
                    last_scanned DATETIME,
                    track_count INTEGER DEFAULT 0
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
                    discovered_at DATETIME,
                    favorite BOOLEAN DEFAULT FALSE,
                    blacklisted BOOLEAN DEFAULT FALSE
                )",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO repositories (owner, name, url, added_at)
                 VALUES ('owner', 'repo', 'https://github.com/owner/repo', ?1)",
                params![Utc::now()],
            )
            .unwrap();
        }

        // Opening should silently apply the ALTER TABLE migration.
        let db = DatabaseManager::from_path(&db_path).unwrap();
        let repos = db.get_repositories().unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].tree_etag, None);

        // And the new column must be writable.
        db.update_tree_etag(repos[0].id, Some("\"etag-after-migration\""))
            .unwrap();
        let after = db.get_repository_by_name("owner", "repo").unwrap();
        assert_eq!(after.tree_etag.as_deref(), Some("\"etag-after-migration\""));
    }

    #[test]
    fn setting_round_trips_and_upserts() {
        let dir = tempdir().unwrap();
        let db = DatabaseManager::from_path(dir.path().join("music.db")).unwrap();

        assert_eq!(db.get_setting("playback_mode").unwrap(), None);

        db.set_setting("playback_mode", "shuffle").unwrap();
        assert_eq!(
            db.get_setting("playback_mode").unwrap().as_deref(),
            Some("shuffle")
        );

        // Setting the same key again overwrites.
        db.set_setting("playback_mode", "sequential").unwrap();
        assert_eq!(
            db.get_setting("playback_mode").unwrap().as_deref(),
            Some("sequential")
        );

        // Distinct keys do not collide.
        db.set_setting("other", "value").unwrap();
        assert_eq!(
            db.get_setting("playback_mode").unwrap().as_deref(),
            Some("sequential")
        );
        assert_eq!(db.get_setting("other").unwrap().as_deref(), Some("value"));
    }

    #[test]
    fn settings_persist_across_reopens() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        {
            let db = DatabaseManager::from_path(&db_path).unwrap();
            db.set_setting("playback_mode", "shuffle").unwrap();
        }

        let reopened = DatabaseManager::from_path(&db_path).unwrap();
        assert_eq!(
            reopened.get_setting("playback_mode").unwrap().as_deref(),
            Some("shuffle")
        );
    }

    #[test]
    fn legacy_database_without_settings_table_gets_migrated() {
        // Simulate a database opened before the settings table existed.
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        {
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
                    downloaded BOOLEAN DEFAULT FALSE
                )",
                [],
            )
            .unwrap();
        }

        let db = DatabaseManager::from_path(&db_path).unwrap();
        assert_eq!(db.get_setting("playback_mode").unwrap(), None);
        db.set_setting("playback_mode", "shuffle").unwrap();
        assert_eq!(
            db.get_setting("playback_mode").unwrap().as_deref(),
            Some("shuffle")
        );
    }
}
