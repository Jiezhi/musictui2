use musictui2::cache::CacheManager;
use tempfile::tempdir;

#[tokio::test]
async fn test_cache_put_and_get() {
    let temp_dir = tempdir().unwrap();

    // Create a new cache manager
    let mut cache = CacheManager::from_dir(temp_dir.path().join("cache"));

    // Set a custom max size
    cache.set_max_size(1024 * 1024); // 1MB

    // Test putting data in cache
    let key = "test_key";
    let data = b"test data";
    let result = cache.put(key, data);
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.exists());

    // Test getting data from cache
    let result = cache.get(key);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().unwrap(), path);

    // Test getting non-existent data
    let result = cache.get("non_existent");
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn test_cache_exists() {
    let temp_dir = tempdir().unwrap();

    // Create a cache manager
    let mut cache = CacheManager::from_dir(temp_dir.path().join("cache"));
    cache.set_max_size(1024 * 1024);

    // Initially not exists
    assert!(!cache.exists("test_key"));

    // Put data
    cache.put("test_key", b"test data").unwrap();
    assert!(cache.exists("test_key"));
}

#[tokio::test]
async fn test_cache_remove() {
    let temp_dir = tempdir().unwrap();
    let mut cache = CacheManager::from_dir(temp_dir.path().join("cache"));
    cache.set_max_size(1024 * 1024);

    // Put data
    cache.put("test_key", b"test data").unwrap();
    assert!(cache.exists("test_key"));

    // Remove data
    let result = cache.remove("test_key");
    assert!(result.is_ok());
    assert!(!cache.exists("test_key"));
}

#[tokio::test]
async fn test_cache_cleanup() {
    let temp_dir = tempdir().unwrap();
    let mut cache = CacheManager::from_dir(temp_dir.path().join("cache"));
    cache.set_max_size(1024); // 1KB limit

    // Put multiple files
    for i in 0..10 {
        let data = vec![b'a'; 200]; // 200 bytes each
        cache.put(&format!("key_{}", i), &data).unwrap();
    }

    // Get cache info before cleanup
    let info = cache.get_cache_info();
    assert!(info.total_size > 0);

    // Cleanup should remove files to stay under limit
    let removed_size = cache.cleanup().unwrap();
    assert!(removed_size > 0);

    // Verify cache is smaller
    let new_info = cache.get_cache_info();
    assert!(new_info.total_size <= cache.get_max_size());
}
