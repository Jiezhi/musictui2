pub mod audio;
pub mod cache;
pub mod cli;
pub mod database;
pub mod events;
pub mod github;
pub mod models;
pub mod tui;

#[cfg(test)]
mod tests {
    #[test]
    fn test_library_compiles() {
        // This test ensures the library can be compiled
        // and all modules are properly connected
    }
}
