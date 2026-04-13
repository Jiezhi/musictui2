pub mod audio;
pub mod models;
pub mod tui;
pub mod cli;
pub mod database;
pub mod github;
pub mod events;
pub mod cache;

#[cfg(test)]
mod tests {
    #[test]
    fn test_library_compiles() {
        // This test ensures the library can be compiled
        // and all modules are properly connected
    }
}