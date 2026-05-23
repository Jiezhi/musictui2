pub mod audio;
pub mod cache;
pub mod cli;
pub mod credentials;
pub mod database;
pub mod errors;
pub mod events;
pub mod github;
pub mod models;
pub mod tui;
pub mod webdav;

#[cfg(test)]
mod tests {
    #[test]
    fn test_library_compiles() {
        // This test ensures the library can be compiled
        // and all modules are properly connected
    }
}
