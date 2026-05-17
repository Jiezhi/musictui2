use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use reqwest::StatusCode;
use tokio::io::AsyncWriteExt;

use crate::cache::{CacheManager, StreamingCacheState};
use crate::database::DatabaseManager;
use crate::github::StreamingTrackDownload;
use crate::models::{Repository, RepositorySource, Track};

#[derive(Clone)]
pub struct WebDavClient {
    client: reqwest::Client,
    username: Option<String>,
    password: Option<String>,
}

pub struct WebDavScanner {
    client: WebDavClient,
    database: Arc<DatabaseManager>,
    cache: Arc<CacheManager>,
}

#[derive(Debug)]
struct WebDavEntry {
    href: String,
    is_collection: bool,
    size: u64,
}

impl WebDavClient {
    pub fn new(username: Option<String>, password: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            username,
            password,
        }
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        let request = self.client.request(method, url);
        if let Some(username) = &self.username {
            request.basic_auth(username, self.password.as_deref())
        } else {
            request
        }
    }

    async fn list_collection(
        &self,
        url: &str,
    ) -> Result<Vec<WebDavEntry>, Box<dyn std::error::Error>> {
        let method = reqwest::Method::from_bytes(b"PROPFIND")?;
        let response = self
            .request(method, url)
            .header("Depth", "1")
            .header(reqwest::header::CONTENT_TYPE, "application/xml")
            .body(
                r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:resourcetype />
    <d:getcontentlength />
  </d:prop>
</d:propfind>"#,
            )
            .send()
            .await?;

        if !response.status().is_success() && response.status() != StatusCode::MULTI_STATUS {
            return Err(format!("WebDAV PROPFIND failed: {}", response.status()).into());
        }

        parse_propfind_response(&response.text().await?)
    }

    pub async fn download(&self, url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let response = self.request(reqwest::Method::GET, url).send().await?;
        if !response.status().is_success() {
            return Err(format!("WebDAV download failed: {}", response.status()).into());
        }

        Ok(response.bytes().await?.to_vec())
    }
}

impl WebDavScanner {
    pub fn new(
        database: Arc<DatabaseManager>,
        cache: Arc<CacheManager>,
        username: Option<String>,
        password: Option<String>,
    ) -> Self {
        Self {
            client: WebDavClient::new(username, password),
            database,
            cache,
        }
    }

    pub async fn add_source(
        &self,
        name: &str,
        url: &str,
        username: Option<String>,
        password: Option<String>,
        cache_enabled: bool,
    ) -> Result<Repository, Box<dyn std::error::Error>> {
        let url = normalize_collection_url(url);
        let repository = Repository {
            id: 0,
            owner: "webdav".to_string(),
            name: name.to_string(),
            url,
            source_type: RepositorySource::WebDav,
            cache_enabled,
            username,
            password,
            added_at: Utc::now(),
            last_scanned: None,
            track_count: 0,
        };

        self.database.save_repository(&repository)?;
        self.database
            .get_repository_by_name("webdav", name)
            .map_err(Into::into)
    }

    pub async fn scan_repository(
        &self,
        repository: &Repository,
    ) -> Result<Vec<Track>, Box<dyn std::error::Error>> {
        let mut tracks = Vec::new();
        let mut queue = vec![repository.url.clone()];

        while let Some(collection_url) = queue.pop() {
            for entry in self.client.list_collection(&collection_url).await? {
                let entry_url = resolve_webdav_url(&repository.url, &entry.href)?;

                if same_collection_url(&entry_url, &collection_url) {
                    continue;
                }

                if entry.is_collection {
                    queue.push(normalize_collection_url(&entry_url));
                    continue;
                }

                let Some(file_name) = file_name_from_url(&entry_url) else {
                    continue;
                };
                let Some(ext) = Path::new(&file_name).extension() else {
                    continue;
                };
                let ext = ext.to_string_lossy().to_lowercase();
                if !is_audio_format(&ext) {
                    continue;
                }

                tracks.push(Track {
                    id: 0,
                    repository_id: repository.id,
                    path: webdav_track_path(&repository.url, &entry_url),
                    name: file_name,
                    format: ext,
                    size: entry.size,
                    duration: None,
                    url: entry_url,
                    local_path: None,
                    downloaded: false,
                    discovered_at: Utc::now(),
                    favorite: false,
                    blacklisted: false,
                });
            }
        }

        for track in &tracks {
            self.database.save_track(track)?;
        }

        Ok(tracks)
    }

    pub async fn download_track(
        &self,
        track: &Track,
        cache_enabled: bool,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        if cache_enabled && self.cache.exists(&track.url) {
            if let Some(path) = self.cache.get(&track.url)? {
                return Ok(path);
            }
        }

        let content = self.client.download(&track.url).await?;
        if cache_enabled {
            return self.cache.put(&track.url, &content);
        }

        let mut file = tempfile::NamedTempFile::new()?;
        std::io::Write::write_all(&mut file, &content)?;
        let (_file, path) = file.keep()?;
        Ok(path)
    }

    pub fn start_streaming_download(
        &self,
        track: Track,
        cache_enabled: bool,
    ) -> Result<StreamingTrackDownload, Box<dyn std::error::Error>> {
        if cache_enabled && self.cache.exists(&track.url) {
            if let Some(path) = self.cache.get(&track.url)? {
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

        let cache_path = if cache_enabled {
            self.cache.path_for_key(&track.url)
        } else {
            tempfile::NamedTempFile::new()?.into_temp_path().keep()?
        };
        let marker_path = if cache_enabled {
            self.cache.incomplete_marker_path_for_key(&track.url)
        } else {
            cache_path.with_extension("part")
        };
        let state = StreamingCacheState::new();
        let client = self.client.clone();
        let database = self.database.clone();
        let download_state = state.clone();

        if let Some(parent) = cache_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let handle_cache_path = cache_path.clone();
        let handle = tokio::spawn(async move {
            let local_path = stream_webdav_to_file(
                client,
                track.url.clone(),
                handle_cache_path,
                marker_path,
                download_state,
            )
            .await?;

            if cache_enabled {
                let mut updated_track = track;
                updated_track.local_path = Some(local_path.clone());
                updated_track.downloaded = true;
                database
                    .save_track(&updated_track)
                    .map_err(|err| err.to_string())?;
            }

            Ok(local_path)
        });

        Ok(StreamingTrackDownload {
            cache_path,
            state,
            handle,
        })
    }
}

async fn stream_webdav_to_file(
    client: WebDavClient,
    url: String,
    path: PathBuf,
    marker_path: PathBuf,
    state: StreamingCacheState,
) -> Result<PathBuf, String> {
    state.reset();
    tokio::fs::write(&marker_path, b"")
        .await
        .map_err(|err| format!("Failed to prepare download marker: {err}"))?;

    let mut response = client
        .request(reqwest::Method::GET, &url)
        .send()
        .await
        .map_err(|err| format!("Failed to start WebDAV download: {err}"))?;
    if !response.status().is_success() {
        let error = format!("WebDAV download failed: {}", response.status());
        state.mark_error(error.clone());
        return Err(error);
    }

    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(|err| format!("Failed to create local file: {err}"))?;
    let mut downloaded_bytes = 0_u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| format!("Failed while reading WebDAV stream: {err}"))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|err| format!("Failed to write local file: {err}"))?;
        downloaded_bytes += chunk.len() as u64;
        state.mark_progress(downloaded_bytes);
    }
    file.flush()
        .await
        .map_err(|err| format!("Failed to flush local file: {err}"))?;

    let _ = tokio::fs::remove_file(&marker_path).await;
    state.mark_complete(downloaded_bytes);
    Ok(path)
}

fn parse_propfind_response(xml: &str) -> Result<Vec<WebDavEntry>, Box<dyn std::error::Error>> {
    let mut entries = Vec::new();
    for response in xml.split("<d:response").skip(1) {
        let href = extract_xml_text(response, "href").unwrap_or_default();
        if href.is_empty() {
            continue;
        }
        let size = extract_xml_text(response, "getcontentlength")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        entries.push(WebDavEntry {
            href,
            is_collection: response.contains("<d:collection")
                || response.contains("<collection")
                || response.contains(":collection"),
            size,
        });
    }

    Ok(entries)
}

fn extract_xml_text(xml: &str, local_name: &str) -> Option<String> {
    let start_suffix = format!(":{local_name}>");
    let end_suffix = format!(":{local_name}>");
    let start = xml
        .find(&start_suffix)
        .map(|index| index + start_suffix.len())?;
    let end = xml[start..]
        .find(&end_suffix)
        .map(|index| start + index - 2)?;
    Some(xml[start..end].trim().to_string())
}

fn normalize_collection_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    }
}

fn resolve_webdav_url(base_url: &str, href: &str) -> Result<String, Box<dyn std::error::Error>> {
    let base = reqwest::Url::parse(base_url)?;
    Ok(base.join(href)?.to_string())
}

fn same_collection_url(left: &str, right: &str) -> bool {
    normalize_collection_url(left) == normalize_collection_url(right)
}

fn file_name_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    parsed
        .path_segments()?
        .rfind(|segment| !segment.is_empty())
        .map(percent_decode)
}

fn webdav_track_path(root_url: &str, track_url: &str) -> String {
    let root = reqwest::Url::parse(root_url).ok();
    let track = reqwest::Url::parse(track_url).ok();

    if let (Some(root), Some(track)) = (root, track) {
        if let Some(path) = track.path().strip_prefix(root.path()) {
            return percent_decode(path.trim_start_matches('/'));
        }
    }

    file_name_from_url(track_url).unwrap_or_else(|| track_url.to_string())
}

fn percent_decode(value: &str) -> String {
    value.replace("%20", " ")
}

fn is_audio_format(ext: &str) -> bool {
    matches!(ext, "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "wma")
}
