use std::sync::Arc;

use chrono::Utc;

use crate::cache::CacheManager;
use crate::database::DatabaseManager;
use crate::models::{Repository, Track};

pub struct GitHubScanner {
    client: GitHubApiClient,
    database: Arc<DatabaseManager>,
    cache: Arc<CacheManager>,
}

#[derive(Debug)]
struct RepoFile {
    name: String,
    path: String,
    r#type: String,
    download_url: Option<String>,
    size: Option<u64>,
}

struct GitHubApiClient {
    #[allow(dead_code)]
    client: reqwest::Client,
    #[allow(dead_code)]
    token: Option<String>,
}

impl GitHubApiClient {
    fn new() -> Self {
        let token = std::env::var("GITHUB_TOKEN").ok();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::USER_AGENT, "musictui2/0.1.0".parse().unwrap());

        // Add authorization if token exists
        if let Some(ref token) = token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("token {}", token).parse().unwrap(),
            );
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .expect("Failed to create HTTP client");

        Self { client, token }
    }

    #[allow(dead_code)]
    pub async fn get_repositories(
        &self,
        owner: &str,
    ) -> Result<Vec<Repository>, Box<dyn std::error::Error>> {
        let url = format!("https://api.github.com/users/{owner}/repos");
        let request = self.client.get(&url);

        // Without authentication, GitHub returns 403 for more than 60 requests/hour
        if self.token.is_none() {
            eprintln!("Warning: No GITHUB_TOKEN set - rate limited to 60 requests/hour");
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(format!("GitHub API error: {}", response.status()).into());
        }

        let repositories: Vec<GitHubRepo> = response.json().await?;

        Ok(repositories
            .into_iter()
            .map(|repo| Repository {
                id: 0,
                owner: owner.to_string(),
                name: repo.name,
                url: repo.html_url,
                added_at: Utc::now(),
                last_scanned: None,
                track_count: 0,
            })
            .collect())
    }

    pub async fn scan_repository(
        &self,
        owner: &str,
        repo_name: &str,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error>> {
        let mut tracks = Vec::new();
        let mut queue = vec![String::new()];

        while let Some(path) = queue.pop() {
            let contents = self.get_repository_contents(owner, repo_name, &path).await?;

            for file in contents {
                if file.r#type == "dir" {
                    queue.push(file.path);
                    continue;
                }

                if file.r#type == "file" {
                    if let Some(ext) = std::path::Path::new(&file.name).extension() {
                        let ext = ext.to_string_lossy().to_lowercase();
                        if is_audio_format(&ext) {
                            if let Some(download_url) = file.download_url {
                                tracks.push(Track {
                                    id: 0,
                                    repository_id: 0,
                                    path: file.path,
                                    name: file.name,
                                    format: ext,
                                    size: file.size.unwrap_or(0),
                                    duration: None,
                                    url: download_url,
                                    local_path: None,
                                    downloaded: false,
                                    discovered_at: Utc::now(),
                                });
                            }
                        }
                    }
                }
            }
        }

        Ok(tracks)
    }

    pub async fn get_file_content(&self, url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let response = self.client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(format!("Failed to download file: {}", response.status()).into());
        }

        Ok(response.bytes().await?.to_vec())
    }

    pub async fn get_repository_contents(
        &self,
        owner: &str,
        repo_name: &str,
        path: &str,
    ) -> Result<Vec<RepoFile>, Box<dyn std::error::Error>> {
        let url = if path.is_empty() {
            format!("https://api.github.com/repos/{owner}/{repo_name}/contents")
        } else {
            format!("https://api.github.com/repos/{owner}/{repo_name}/contents/{path}")
        };

        let response = self.client.get(&url).send().await?;

        if response.status() == reqwest::StatusCode::FORBIDDEN {
            let error_text = response.text().await.unwrap_or_default();
            if error_text.contains("rate limit") {
                return Err("GitHub API rate limit exceeded. Set GITHUB_TOKEN to increase limit".into());
            } else {
                return Err("GitHub API access forbidden. Please check your access or set GITHUB_TOKEN".into());
            }
        }

        if !response.status().is_success() {
            return Err(format!("GitHub API error: {}", response.status()).into());
        }

        let contents: Vec<GitHubContent> = response.json().await?;

        Ok(contents
            .into_iter()
            .map(|content| RepoFile {
                name: content.name,
                path: content.path,
                r#type: content.r#type,
                download_url: content.download_url,
                size: content.size,
            })
            .collect())
    }
}

impl GitHubScanner {
    pub fn new(database: Arc<DatabaseManager>, cache: Arc<CacheManager>) -> Self {
        Self {
            client: GitHubApiClient::new(),
            database,
            cache,
        }
    }

    pub async fn scan_repository(
        &self,
        owner: &str,
        repo_name: &str,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error>> {
        let repository = match self.database.get_repository_by_name(owner, repo_name) {
            Ok(repo) => repo,
            Err(_) => {
                let repository = Repository {
                    id: 0,
                    owner: owner.to_string(),
                    name: repo_name.to_string(),
                    url: format!("https://github.com/{owner}/{repo_name}"),
                    added_at: Utc::now(),
                    last_scanned: None,
                    track_count: 0,
                };

                self.database.save_repository(&repository)?;
                self.database.get_repository_by_name(owner, repo_name)?
            }
        };

        let mut tracks = self.client.scan_repository(owner, repo_name).await?;

        for track in &mut tracks {
            track.repository_id = repository.id;
        }

        for track in &tracks {
            self.database.save_track(track)?;
        }

        Ok(tracks)
    }

    pub async fn download_track(
        &self,
        track: &Track,
    ) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
        if self.cache.exists(&track.url) {
            if let Some(path) = self.cache.get(&track.url)? {
                return Ok(path);
            }
        }

        let content = self.client.get_file_content(&track.url).await?;
        let local_path = self.cache.put(&track.url, &content)?;

        let mut updated_track = track.clone();
        updated_track.local_path = Some(local_path.clone());
        updated_track.downloaded = true;
        self.database.save_track(&updated_track)?;

        Ok(local_path)
    }
}

#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
struct GitHubRepo {
    name: String,
    html_url: String,
}

#[derive(Debug, serde::Deserialize)]
struct GitHubContent {
    name: String,
    path: String,
    r#type: String,
    download_url: Option<String>,
    size: Option<u64>,
}

fn is_audio_format(ext: &str) -> bool {
    matches!(ext, "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "wma")
}
