use std::sync::Arc;

use chrono::Utc;

use crate::cache::CacheManager;
use crate::database::DatabaseManager;
use crate::events::EventBus;
use crate::github::GitHubScanner;
use crate::models::{Repository, RepositorySource, Track};
use crate::webdav::WebDavScanner;

pub struct Cli {
    _event_bus: EventBus,
    github_scanner: Arc<GitHubScanner>,
    database: Arc<DatabaseManager>,
    cache: Arc<CacheManager>,
}

impl Cli {
    pub fn new(event_bus: EventBus) -> Self {
        let database = Arc::new(DatabaseManager::new());
        let cache = Arc::new(CacheManager::new());
        let github_scanner = Arc::new(GitHubScanner::new(database.clone(), cache.clone()));

        Self {
            _event_bus: event_bus,
            github_scanner,
            database,
            cache,
        }
    }

    pub async fn add_repository(&self, repo_url: &str) -> Result<(), Box<dyn std::error::Error>> {
        let (owner, repo_name) = parse_repository_url(repo_url)?;

        let repository = Repository {
            id: 0,
            owner: owner.clone(),
            name: repo_name.clone(),
            url: format!("https://github.com/{owner}/{repo_name}"),
            source_type: RepositorySource::GitHub,
            cache_enabled: true,
            username: None,
            password: None,
            added_at: Utc::now(),
            last_scanned: None,
            track_count: 0,
        };

        self.database.save_repository(&repository)?;
        self.scan_repository(&format!("{owner}/{repo_name}"))
            .await?;

        Ok(())
    }

    pub async fn add_webdav_source(
        &self,
        name: &str,
        url: &str,
        username: Option<&str>,
        password: Option<&str>,
        cache_enabled: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let scanner = WebDavScanner::new(
            self.database.clone(),
            self.cache.clone(),
            username.map(ToString::to_string),
            password.map(ToString::to_string),
        );
        let repository = scanner
            .add_source(
                name,
                url,
                username.map(ToString::to_string),
                password.map(ToString::to_string),
                cache_enabled,
            )
            .await?;
        let tracks = scanner.scan_repository(&repository).await?;
        self.database.update_last_scanned_by_id(repository.id)?;

        println!("Found {} audio files:", tracks.len());
        for track in tracks {
            println!("  - {} (ID: {})", track.path, track.id);
        }

        Ok(())
    }

    pub async fn list_repositories(&self) -> Result<Vec<Repository>, Box<dyn std::error::Error>> {
        Ok(self.database.get_repositories()?)
    }

    pub async fn remove_repository(
        &self,
        repo_identifier: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(repository) = self.find_repository(repo_identifier)? {
            self.github_scanner.delete_repository(repository.id)?;
            return Ok(());
        }

        let (owner, repo_name) = parse_repository_url(repo_identifier)?;
        self.github_scanner
            .delete_repository_by_name(&owner, &repo_name)?;

        Ok(())
    }

    pub async fn scan_repository(
        &self,
        repo_identifier: &str,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error>> {
        if let Some(repository) = self.find_repository(repo_identifier)? {
            if repository.source_type == RepositorySource::WebDav {
                let scanner = WebDavScanner::new(
                    self.database.clone(),
                    self.cache.clone(),
                    repository.username.clone(),
                    repository.password.clone(),
                );
                let tracks = scanner.scan_repository(&repository).await?;
                self.database.update_last_scanned_by_id(repository.id)?;
                return Ok(tracks);
            }
        }

        let (owner, repo_name) = parse_repository_url(repo_identifier)?;
        let tracks = self
            .github_scanner
            .scan_repository(&owner, &repo_name)
            .await?;

        self.database.update_last_scanned(&owner, &repo_name)?;

        Ok(tracks)
    }

    pub async fn download_track(&self, track_id: i64) -> Result<Track, Box<dyn std::error::Error>> {
        let track = self.database.get_track_by_id(track_id)?;
        let repository = self.database.get_repository_by_id(track.repository_id)?;
        let local_path = match repository.source_type {
            RepositorySource::GitHub => self.github_scanner.download_track(&track).await?,
            RepositorySource::WebDav => {
                let scanner = WebDavScanner::new(
                    self.database.clone(),
                    self.cache.clone(),
                    repository.username.clone(),
                    repository.password.clone(),
                );
                scanner
                    .download_track(&track, repository.cache_enabled)
                    .await?
            }
        };

        // Update track with local path
        let mut updated_track = track.clone();
        updated_track.local_path = Some(local_path);
        updated_track.downloaded = repository.cache_enabled;
        self.database.save_track(&updated_track)?;

        Ok(updated_track)
    }

    pub async fn list_tracks(
        &self,
        repo_identifier: Option<&str>,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error>> {
        if let Some(repo_identifier) = repo_identifier {
            if let Some(repository) = self.find_repository(repo_identifier)? {
                return Ok(self.database.get_tracks_by_repo(repository.id)?);
            }

            let (owner, repo_name) = parse_repository_url(repo_identifier)?;
            Ok(self.database.get_tracks_by_repo_name(&owner, &repo_name)?)
        } else {
            Ok(self.database.get_all_tracks()?)
        }
    }

    pub async fn list_favorite_tracks(
        &self,
        repo_identifier: Option<&str>,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error>> {
        let tracks = if repo_identifier.is_some() {
            self.list_tracks(repo_identifier).await?
        } else {
            self.database.get_favorite_tracks()?
        };

        Ok(tracks
            .into_iter()
            .filter(|track| track.favorite && !track.blacklisted)
            .collect())
    }

    pub async fn list_blacklisted_tracks(
        &self,
        repo_identifier: Option<&str>,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error>> {
        let tracks = if repo_identifier.is_some() {
            self.list_tracks(repo_identifier).await?
        } else {
            self.database.get_blacklisted_tracks()?
        };

        Ok(tracks
            .into_iter()
            .filter(|track| track.blacklisted)
            .collect())
    }

    pub async fn set_track_favorite(
        &self,
        track_id: i64,
        favorite: bool,
    ) -> Result<Track, Box<dyn std::error::Error>> {
        let track = self.database.get_track_by_id(track_id)?;
        if track.blacklisted && favorite {
            return Err("Blacklisted tracks cannot be favorited. Unblock the track first.".into());
        }

        self.database.set_track_favorite(track_id, favorite)?;
        Ok(self.database.get_track_by_id(track_id)?)
    }

    pub async fn set_track_blacklisted(
        &self,
        track_id: i64,
        blacklisted: bool,
    ) -> Result<Track, Box<dyn std::error::Error>> {
        self.database.get_track_by_id(track_id)?;
        self.database.set_track_blacklisted(track_id, blacklisted)?;
        Ok(self.database.get_track_by_id(track_id)?)
    }

    fn find_repository(
        &self,
        repo_identifier: &str,
    ) -> Result<Option<Repository>, Box<dyn std::error::Error>> {
        if let Ok(id) = repo_identifier.parse::<i64>() {
            return match self.database.get_repository_by_id(id) {
                Ok(repository) => Ok(Some(repository)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(err) => Err(err.into()),
            };
        }

        match self
            .database
            .get_repository_by_name("webdav", repo_identifier)
        {
            Ok(repository) => Ok(Some(repository)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

fn parse_repository_url(url: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    let normalized = url
        .trim()
        .trim_end_matches('/')
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("github.com/");

    let parts: Vec<&str> = normalized.split('/').collect();

    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err("Invalid repository URL format. Expected owner/repo or GitHub URL".into());
    }

    Ok((parts[0].to_string(), parts[1].to_string()))
}
