use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use reqwest::StatusCode;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

use crate::cache::{CacheManager, StreamingCacheState};
use crate::database::DatabaseManager;
use crate::models::{Repository, Track};

const MAX_GITHUB_REQUEST_ATTEMPTS: u32 = 3;

pub struct GitHubScanner {
    client: GitHubApiClient,
    database: Arc<DatabaseManager>,
    cache: Arc<CacheManager>,
}

#[derive(Debug)]
pub struct StreamingTrackDownload {
    pub cache_path: PathBuf,
    pub state: StreamingCacheState,
    pub handle: JoinHandle<Result<PathBuf, String>>,
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
        headers.insert(
            reqwest::header::USER_AGENT,
            "musictui2/0.1.0".parse().unwrap(),
        );

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

        let repositories: Vec<GitHubRepo> = self
            .send_github_json_request(&url, "GitHub API error")
            .await?;

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
            let contents = self
                .get_repository_contents(owner, repo_name, &path)
                .await?;

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
        let mut last_error = None;

        for attempt in 1..=MAX_GITHUB_REQUEST_ATTEMPTS {
            match self.client.get(url).send().await {
                Ok(response) => {
                    let status = response.status();

                    if status.is_success() {
                        match response.bytes().await {
                            Ok(bytes) => return Ok(bytes.to_vec()),
                            Err(err) => {
                                if attempt == MAX_GITHUB_REQUEST_ATTEMPTS {
                                    return Err(err.into());
                                }
                                last_error = Some(err);
                            }
                        }
                    } else if should_retry_status(status) && attempt < MAX_GITHUB_REQUEST_ATTEMPTS {
                        last_error = None;
                    } else {
                        return Err(format!("Failed to download file: {}", status).into());
                    }
                }
                Err(err) => {
                    if attempt == MAX_GITHUB_REQUEST_ATTEMPTS {
                        return Err(err.into());
                    }
                    last_error = Some(err);
                }
            }

            sleep(retry_delay(attempt)).await;
        }

        Err(last_error
            .map(|err| format!("Failed to download file after retries: {err}"))
            .unwrap_or_else(|| "Failed to download file after retries".to_string())
            .into())
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

        let contents: Vec<GitHubContent> = self
            .send_github_json_request(&url, "GitHub API error")
            .await?;

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

    async fn send_github_json_request<T>(
        &self,
        url: &str,
        error_prefix: &str,
    ) -> Result<T, Box<dyn std::error::Error>>
    where
        T: serde::de::DeserializeOwned,
    {
        if self.token.is_none() {
            eprintln!("Warning: No GITHUB_TOKEN set - rate limited to 60 requests/hour");
        }

        let mut last_error = None;

        for attempt in 1..=MAX_GITHUB_REQUEST_ATTEMPTS {
            match self.client.get(url).send().await {
                Ok(response) => {
                    let status = response.status();

                    if status == StatusCode::FORBIDDEN {
                        return Err(github_forbidden_error(response).await);
                    }

                    if status.is_success() {
                        match response.json::<T>().await {
                            Ok(value) => return Ok(value),
                            Err(err) => {
                                if attempt == MAX_GITHUB_REQUEST_ATTEMPTS {
                                    return Err(err.into());
                                }
                                last_error = Some(err);
                            }
                        }
                    } else if should_retry_status(status) && attempt < MAX_GITHUB_REQUEST_ATTEMPTS {
                        last_error = None;
                    } else {
                        return Err(format!("{error_prefix}: {status}").into());
                    }
                }
                Err(err) => {
                    if attempt == MAX_GITHUB_REQUEST_ATTEMPTS {
                        return Err(err.into());
                    }
                    last_error = Some(err);
                }
            }

            sleep(retry_delay(attempt)).await;
        }

        Err(last_error
            .map(|err| format!("{error_prefix} after retries: {err}"))
            .unwrap_or_else(|| format!("{error_prefix} after retries"))
            .into())
    }
}

async fn github_forbidden_error(response: reqwest::Response) -> Box<dyn std::error::Error> {
    let error_text = response.text().await.unwrap_or_default();
    if error_text.contains("rate limit") {
        "GitHub API rate limit exceeded. Set GITHUB_TOKEN to increase limit".into()
    } else {
        "GitHub API access forbidden. Please check your access or set GITHUB_TOKEN".into()
    }
}

fn should_retry_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn retry_delay(attempt: u32) -> Duration {
    Duration::from_millis(250 * 2_u64.pow(attempt - 1))
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
                let mut updated_track = track.clone();
                updated_track.local_path = Some(path.clone());
                updated_track.downloaded = true;
                self.database.save_track(&updated_track)?;
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

    pub fn delete_repository(
        &self,
        repository_id: i64,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let tracks = self.database.get_tracks_by_repo(repository_id)?;

        for track in &tracks {
            self.cache.remove(&track.url)?;
        }

        self.database.delete_repository(repository_id)?;
        Ok(tracks.len())
    }

    pub fn delete_repository_by_name(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let repository = self.database.get_repository_by_name(owner, name)?;
        self.delete_repository(repository.id)
    }

    pub fn start_streaming_download(
        &self,
        track: Track,
    ) -> Result<StreamingTrackDownload, Box<dyn std::error::Error>> {
        if self.cache.exists(&track.url) {
            if let Some(path) = self.cache.get(&track.url)? {
                let mut updated_track = track.clone();
                updated_track.local_path = Some(path.clone());
                updated_track.downloaded = true;
                self.database.save_track(&updated_track)?;

                let state = StreamingCacheState::new();
                let downloaded_bytes = std::fs::metadata(&path).map(|metadata| metadata.len())?;
                state.mark_complete(downloaded_bytes);
                let cache_path = path.clone();
                let handle = tokio::spawn(async move { Ok(path) });

                return Ok(StreamingTrackDownload {
                    cache_path,
                    state,
                    handle,
                });
            }
        }

        let cache_path = self.cache.path_for_key(&track.url);
        let marker_path = self.cache.incomplete_marker_path_for_key(&track.url);
        let state = StreamingCacheState::new();
        let client = self.client.client.clone();
        let database = self.database.clone();
        let download_state = state.clone();

        if let Some(parent) = cache_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let handle_cache_path = cache_path.clone();
        let handle = tokio::spawn(async move {
            let local_path = stream_track_to_cache(
                client,
                track.url.clone(),
                handle_cache_path,
                marker_path,
                download_state.clone(),
            )
            .await?;

            let mut updated_track = track;
            updated_track.local_path = Some(local_path.clone());
            updated_track.downloaded = true;
            database
                .save_track(&updated_track)
                .map_err(|err| err.to_string())?;

            Ok(local_path)
        });

        Ok(StreamingTrackDownload {
            cache_path,
            state,
            handle,
        })
    }
}

async fn stream_track_to_cache(
    client: reqwest::Client,
    url: String,
    cache_path: PathBuf,
    marker_path: PathBuf,
    state: StreamingCacheState,
) -> Result<PathBuf, String> {
    state.reset();

    if let Err(err) = tokio::fs::write(&marker_path, b"").await {
        let error = format!("Failed to prepare cache marker: {err}");
        state.mark_error(error.clone());
        return Err(error);
    }

    match stream_track_to_cache_once(&client, &url, &cache_path, &state).await {
        Ok(downloaded_bytes) => {
            let _ = tokio::fs::remove_file(&marker_path).await;
            state.mark_complete(downloaded_bytes);
            Ok(cache_path)
        }
        Err(err) => {
            state.mark_error(err.clone());
            Err(err)
        }
    }
}

async fn stream_track_to_cache_once(
    client: &reqwest::Client,
    url: &str,
    cache_path: &PathBuf,
    state: &StreamingCacheState,
) -> Result<u64, String> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|err| format!("Failed to start download: {err}"))?;

    let status = response.status();
    if status == StatusCode::FORBIDDEN {
        return Err(github_forbidden_error(response).await.to_string());
    }

    if !status.is_success() {
        return Err(format!("Failed to download file: {status}"));
    }

    let mut file = tokio::fs::File::create(cache_path)
        .await
        .map_err(|err| format!("Failed to create cache file: {err}"))?;
    let mut downloaded_bytes = 0_u64;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| format!("Failed while reading download stream: {err}"))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|err| format!("Failed to write cache file: {err}"))?;
        downloaded_bytes += chunk.len() as u64;
        state.mark_progress(downloaded_bytes);
    }

    file.flush()
        .await
        .map_err(|err| format!("Failed to flush cache file: {err}"))?;

    Ok(downloaded_bytes)
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
