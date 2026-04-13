use std::path::{Path, PathBuf};
use std::fs;
use sha2::{Sha256, Digest};

pub struct CacheManager {
    cache_dir: PathBuf,
    max_size: u64,
    cache: Box<dyn Cache>,
}

trait Cache {
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
        // Create SHA256 hash of key for filename
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        self.base_dir.join(&hash)
    }
}

impl Cache for FileCache {
    fn get(&self, key: &str) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
        let file_path = self.get_file_path(key);

        if file_path.exists() {
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
        Ok(file_path)
    }

    fn exists(&self, key: &str) -> bool {
        let file_path = self.get_file_path(key);
        file_path.exists()
    }

    fn remove(&self, key: &str) -> Result<(), Box<dyn std::error::Error>> {
        let file_path = self.get_file_path(key);

        if file_path.exists() {
            fs::remove_file(file_path)?;
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
        files.sort_by(|a, b| a.1.cmp(&b.1));

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

        let cache_dir = config_dir.join("cache");
        fs::create_dir_all(&cache_dir).ok();

        let cache = Box::new(FileCache::new(cache_dir.clone()));

        Self {
            cache_dir,
            max_size: 1024 * 1024 * 1024, // 1GB default
            cache,
        }
    }

    #[allow(dead_code)]
    pub fn get_cache_dir(&self) -> &Path {
        &self.cache_dir
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
        self.cache.cleanup(self.max_size)
    }

    #[allow(dead_code)]
    pub fn set_max_size(&mut self, size: u64) {
        self.max_size = size;
    }

    #[allow(dead_code)]
    pub fn get_cache_info(&self) -> CacheInfo {
        let mut total_size = 0u64;
        let mut file_count = 0u64;

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
            max_size: self.max_size,
        }
    }

    #[allow(dead_code)]
    pub fn get_max_size(&self) -> u64 {
        self.max_size
    }

    #[allow(dead_code)]
    pub fn get_cache_dir_path(&self) -> &PathBuf {
        &self.cache_dir
    }
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