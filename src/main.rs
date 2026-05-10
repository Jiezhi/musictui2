use clap::{Parser, Subcommand};
use std::path::PathBuf;

fn load_env_file() {
    if let Ok(env) = std::fs::read_to_string(".env") {
        for line in env.lines() {
            if let Some((key, value)) = line.split_once('=') {
                std::env::set_var(key.trim(), value.trim());
            }
        }
    }
}

mod audio;
mod cache;
mod cli;
mod database;
mod events;
mod github;
pub mod models;
mod tui;

use cli::Cli;
use events::EventBus;

#[derive(Parser)]
#[command(name = "musictui2")]
#[command(
    about = "Cross-platform terminal music player that streams audio files directly from GitHub repositories"
)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a GitHub repository to the library
    Add {
        /// GitHub repository URL or owner/repo format
        repository: String,
    },
    /// List all added repositories
    List,
    /// Remove a repository from the library
    Remove {
        /// Repository ID or owner/repo format
        repository: String,
    },
    /// Scan a repository for audio files
    Scan {
        /// Repository to scan
        repository: String,
    },
    /// Update repository scan
    UpdateScan {
        /// Repository to scan
        repository: String,
    },
    /// Download a specific track
    Download {
        /// Track ID to download
        track_id: i64,
    },
    /// List all tracks
    ListTracks {
        /// Repository to filter tracks (optional)
        #[arg(short, long)]
        repository: Option<String>,
    },
    /// Launch TUI mode
    Tui,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env file at startup
    load_env_file();

    let args = Args::parse();

    let event_bus = EventBus::new();
    let _config_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("musictui2");

    let cli = Cli::new(event_bus.clone());

    match args.command {
        Commands::Add { repository } => {
            cli.add_repository(&repository).await?;
            println!("Added repository: {repository}");
        }
        Commands::List => {
            let repos = cli.list_repositories().await?;
            for repo in repos {
                println!("{} - {}", repo.id, repo.name);
            }
        }
        Commands::Remove { repository } => {
            cli.remove_repository(&repository).await?;
            println!("Removed repository: {repository}");
        }
        Commands::Scan { repository } => {
            let tracks = cli.scan_repository(&repository).await?;
            println!("Found {} audio files:", tracks.len());
            for track in tracks {
                println!("  - {} (ID: {})", track.path, track.id);
            }
        }
        Commands::UpdateScan { repository } => {
            let tracks = cli.scan_repository(&repository).await?;
            println!(
                "Updated scan for {}: found {} audio files",
                repository,
                tracks.len()
            );
            for track in tracks {
                println!("  - {} (ID: {})", track.path, track.id);
            }
        }
        Commands::Download { track_id } => {
            let track = cli.download_track(track_id).await?;
            println!("Downloaded: {} to {:?}", track.name, track.local_path);
        }
        Commands::ListTracks { repository } => {
            let tracks = cli.list_tracks(repository.as_deref()).await?;
            println!("Found {} tracks:", tracks.len());
            for track in tracks {
                let status = if track.downloaded {
                    "Downloaded"
                } else {
                    "Not downloaded"
                };
                println!("  - {} (ID: {}) - {}", track.name, track.id, status);
            }
        }
        Commands::Tui => {
            tui::run(event_bus).await?;
        }
    }

    Ok(())
}
