use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub struct CacheManager {
    cache_dir: PathBuf,
    max_size: AtomicU64,
    cache: Arc<dyn Cache>,
}

trait Cache: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<PathBuf>, Box<dyn std::error::Error>>;
    fn put(&self, key: &str, data: &[u8]) -> Result<PathBuf, Box<dyn std::error::Error>>;
    fn exists(&self, key: &str) -> bool;
    fn remove(&self, key: &str) -> Result<(), Box<dyn std::error::Error>>;
    fn cleanup(&self, max_size: u64) -> Result<u64, Box<dyn std::error::Error>>;
}

struct FileCache {
    base_dir: PathBuf,
}

impl FileCache {
    fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn get_file_path(&self, key: &str) -> PathBuf {
        cache_path_for_key(&self.base_dir, key)
    }

    fn get_incomplete_marker_path(&self, key: &str) -> PathBuf {
        incomplete_marker_path_for_key(&self.base_dir, key)
    }
}

impl Cache for FileCache {
    fn get(&self, key: &str) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
        let file_path = self.get_file_path(key);

        if file_path.exists() && !self.get_incomplete_marker_path(key).exists() {
            Ok(Some(file_path))
        } else {
            Ok(None)
        }
    }

    fn put(&self, key: &str, data: &[u8]) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let file_path = self.get_file_path(key);

        // Ensure parent directory exists
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&file_path, data)?;
        let marker_path = self.get_incomplete_marker_path(key);
        if marker_path.exists() {
            fs::remove_file(marker_path)?;
        }
        Ok(file_path)
    }

    fn exists(&self, key: &str) -> bool {
        let file_path = self.get_file_path(key);
        file_path.exists() && !self.get_incomplete_marker_path(key).exists()
    }

    fn remove(&self, key: &str) -> Result<(), Box<dyn std::error::Error>> {
        let file_path = self.get_file_path(key);
        let marker_path = self.get_incomplete_marker_path(key);

        if file_path.exists() {
            fs::remove_file(file_path)?;
        }

        if marker_path.exists() {
            fs::remove_file(marker_path)?;
        }

        Ok(())
    }

    fn cleanup(&self, max_size: u64) -> Result<u64, Box<dyn std::error::Error>> {
        let mut total_size = 0u64;
        let mut files: Vec<_> = std::fs::read_dir(&self.base_dir)?
            .filter_map(|entry| {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_file() {
                        if let Ok(metadata) = path.metadata() {
                            Some((path, metadata.len()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        // Sort by last modified time (oldest first)
        files.sort_by_key(|file| file.1);

        let mut removed_size = 0u64;
        for (path, size) in files {
            if total_size + size > max_size {
                fs::remove_file(path)?;
                removed_size += size;
            } else {
                total_size += size;
            }
        }

        Ok(removed_size)
    }
}

impl CacheManager {
    pub fn new() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("musictui2");

        Self::from_dir(config_dir.join("cache"))
    }

    pub fn from_dir(cache_dir: impl Into<PathBuf>) -> Self {
        let cache_dir = cache_dir.into();
        fs::create_dir_all(&cache_dir).ok();

        let cache = Arc::new(FileCache::new(cache_dir.clone()));

        Self {
            cache_dir,
            max_size: AtomicU64::new(1024 * 1024 * 1024), // 1GB default
            cache,
        }
    }

    #[allow(dead_code)]
    pub fn get_cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn path_for_key(&self, key: &str) -> PathBuf {
        cache_path_for_key(&self.cache_dir, key)
    }

    pub fn incomplete_marker_path_for_key(&self, key: &str) -> PathBuf {
        incomplete_marker_path_for_key(&self.cache_dir, key)
    }

    #[allow(dead_code)]
    pub fn get(&self, key: &str) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
        self.cache.get(key)
    }

    #[allow(dead_code)]
    pub fn put(&self, key: &str, data: &[u8]) -> Result<PathBuf, Box<dyn std::error::Error>> {
        self.cache.put(key, data)
    }

    #[allow(dead_code)]
    pub fn exists(&self, key: &str) -> bool {
        self.cache.exists(key)
    }

    #[allow(dead_code)]
    pub fn remove(&self, key: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.cache.remove(key)
    }

    #[allow(dead_code)]
    pub fn cleanup(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let max_size = self.max_size.load(Ordering::Relaxed);
        self.cache.cleanup(max_size)
    }

    #[allow(dead_code)]
    pub fn set_max_size(&mut self, size: u64) {
        self.max_size.store(size, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub fn get_cache_info(&self) -> CacheInfo {
        let mut total_size = 0u64;
        let mut file_count = 0u64;
        let max_size = self.max_size.load(Ordering::Relaxed);

        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        total_size += metadata.len();
                        file_count += 1;
                    }
                }
            }
        }

        CacheInfo {
            total_size,
            file_count,
            max_size,
        }
    }

    #[allow(dead_code)]
    pub fn get_max_size(&self) -> u64 {
        self.max_size.load(Ordering::Relaxed)
    }

    #[allow(dead_code)]
    pub fn get_cache_dir_path(&self) -> &PathBuf {
        &self.cache_dir
    }
}

fn cache_path_for_key(base_dir: &Path, key: &str) -> PathBuf {
    // Create SHA256 hash of key for filename.
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    base_dir.join(hash)
}

fn incomplete_marker_path_for_key(base_dir: &Path, key: &str) -> PathBuf {
    let mut path = cache_path_for_key(base_dir, key);
    let Some(file_name) = path.file_name().map(|name| name.to_owned()) else {
        return base_dir.join("streaming.part");
    };

    path.set_file_name(format!("{}.part", file_name.to_string_lossy()));
    path
}

#[derive(Clone, Debug)]
pub struct StreamingCacheState {
    inner: Arc<StreamingCacheShared>,
}

#[derive(Debug)]
struct StreamingCacheShared {
    state: Mutex<StreamingCacheProgress>,
    changed: Condvar,
}

#[derive(Debug)]
struct StreamingCacheProgress {
    downloaded_bytes: u64,
    complete: bool,
    error: Option<String>,
}

impl StreamingCacheState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(StreamingCacheShared {
                state: Mutex::new(StreamingCacheProgress {
                    downloaded_bytes: 0,
                    complete: false,
                    error: None,
                }),
                changed: Condvar::new(),
            }),
        }
    }

    pub fn downloaded_bytes(&self) -> u64 {
        self.inner
            .state
            .lock()
            .map(|state| state.downloaded_bytes)
            .unwrap_or_default()
    }

    pub fn is_complete(&self) -> bool {
        self.inner
            .state
            .lock()
            .map(|state| state.complete)
            .unwrap_or(false)
    }

    pub fn reset(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.downloaded_bytes = 0;
            state.complete = false;
            state.error = None;
            self.inner.changed.notify_all();
        }
    }

    pub fn mark_progress(&self, downloaded_bytes: u64) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.downloaded_bytes = downloaded_bytes;
            self.inner.changed.notify_all();
        }
    }

    pub fn mark_complete(&self, downloaded_bytes: u64) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.downloaded_bytes = downloaded_bytes;
            state.complete = true;
            state.error = None;
            self.inner.changed.notify_all();
        }
    }

    pub fn mark_error(&self, error: impl Into<String>) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.error = Some(error.into());
            self.inner.changed.notify_all();
        }
    }

    pub fn wait_for_bytes(&self, min_bytes: u64) -> Result<(), String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "Streaming cache state is poisoned".to_string())?;

        loop {
            if let Some(error) = &state.error {
                return Err(error.clone());
            }

            if state.downloaded_bytes >= min_bytes || state.complete {
                return Ok(());
            }

            state = self
                .inner
                .changed
                .wait(state)
                .map_err(|_| "Streaming cache state is poisoned".to_string())?;
        }
    }

    fn wait_for_readable_position(&self, position: u64) -> io::Result<Option<u64>> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| io::Error::other("Streaming cache state is poisoned"))?;

        loop {
            if let Some(error) = &state.error {
                return Err(io::Error::other(error.clone()));
            }

            if position < state.downloaded_bytes {
                return Ok(Some(state.downloaded_bytes));
            }

            if state.complete {
                return Ok(None);
            }

            state = self
                .inner
                .changed
                .wait(state)
                .map_err(|_| io::Error::other("Streaming cache state is poisoned"))?;
        }
    }

    pub fn wait_until_complete(&self) -> io::Result<u64> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| io::Error::other("Streaming cache state is poisoned"))?;

        loop {
            if let Some(error) = &state.error {
                return Err(io::Error::other(error.clone()));
            }

            if state.complete {
                return Ok(state.downloaded_bytes);
            }

            state = self
                .inner
                .changed
                .wait_timeout(state, Duration::from_millis(250))
                .map_err(|_| io::Error::other("Streaming cache state is poisoned"))?
                .0;
        }
    }
}

impl Default for StreamingCacheState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct GrowingFileReader {
    file: File,
    position: u64,
    state: StreamingCacheState,
}

impl GrowingFileReader {
    pub fn open(path: impl AsRef<Path>, state: StreamingCacheState) -> io::Result<Self> {
        Ok(Self {
            file: File::open(path)?,
            position: 0,
            state,
        })
    }
}

impl Read for GrowingFileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let Some(downloaded_bytes) = self.state.wait_for_readable_position(self.position)? else {
            return Ok(0);
        };

        let available = (downloaded_bytes - self.position).min(buf.len() as u64) as usize;
        self.file.seek(SeekFrom::Start(self.position))?;
        let bytes_read = self.file.read(&mut buf[..available])?;
        self.position += bytes_read as u64;

        Ok(bytes_read)
    }
}

impl Seek for GrowingFileReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next_position = match pos {
            SeekFrom::Start(position) => position,
            SeekFrom::Current(offset) => checked_seek_position(self.position, offset)?,
            SeekFrom::End(offset) => {
                checked_seek_position(self.state.wait_until_complete()?, offset)?
            }
        };

        self.position = next_position;
        Ok(self.position)
    }
}

fn checked_seek_position(base: u64, offset: i64) -> io::Result<u64> {
    if offset >= 0 {
        base.checked_add(offset as u64)
    } else {
        base.checked_sub(offset.unsigned_abs())
    }
    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid seek position"))
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CacheInfo {
    #[allow(dead_code)]
    pub total_size: u64,
    #[allow(dead_code)]
    pub file_count: u64,
    #[allow(dead_code)]
    pub max_size: u64,
}

impl Default for CacheManager {
    fn default() -> Self {
        Self::new()
    }
}
