use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod audio;
mod cache;
mod cli;
mod credentials;
mod database;
mod errors;
mod events;
mod github;
pub mod models;
mod tui;
mod webdav;

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
    /// Add a WebDAV music source to the library
    AddWebdav {
        /// Display name for the WebDAV source
        name: String,
        /// WebDAV collection URL
        url: String,
        /// Username for basic authentication
        #[arg(long)]
        username: Option<String>,
        /// Password for basic authentication
        #[arg(long)]
        password: Option<String>,
        /// Cache WebDAV tracks after playback/download
        #[arg(long)]
        cache: bool,
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
        /// Show only favorite tracks
        #[arg(long, conflicts_with = "blacklisted")]
        favorites: bool,
        /// Show only blacklisted tracks
        #[arg(long)]
        blacklisted: bool,
    },
    /// Mark a track as favorite
    Favorite {
        /// Track ID to favorite
        track_id: i64,
    },
    /// Remove a track from favorites
    Unfavorite {
        /// Track ID to unfavorite
        track_id: i64,
    },
    /// Hide a track from normal playback lists
    Blacklist {
        /// Track ID to blacklist
        track_id: i64,
    },
    /// Restore a blacklisted track
    Unblacklist {
        /// Track ID to restore
        track_id: i64,
    },
    /// Launch TUI mode
    Tui,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env file at startup (silently ignored if missing).
    let _ = dotenv::dotenv();

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
        Commands::AddWebdav {
            name,
            url,
            username,
            password,
            cache,
        } => {
            cli.add_webdav_source(&name, &url, username.as_deref(), password.as_deref(), cache)
                .await?;
            let cache_status = if cache { "enabled" } else { "disabled" };
            println!("Added WebDAV source: {name} (cache {cache_status})");
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
        Commands::ListTracks {
            repository,
            favorites,
            blacklisted,
        } => {
            let tracks: Vec<_> = if favorites {
                cli.list_favorite_tracks(repository.as_deref()).await?
            } else if blacklisted {
                cli.list_blacklisted_tracks(repository.as_deref()).await?
            } else {
                cli.list_tracks(repository.as_deref())
                    .await?
                    .into_iter()
                    .filter(|track| !track.blacklisted)
                    .collect()
            };

            println!("Found {} tracks:", tracks.len());
            for track in tracks {
                let mut status = Vec::new();
                status.push(if track.downloaded {
                    "Downloaded"
                } else {
                    "Not downloaded"
                });
                if track.favorite {
                    status.push("Favorite");
                }
                if track.blacklisted {
                    status.push("Blacklisted");
                }
                println!(
                    "  - {} (ID: {}) - {}",
                    track.name,
                    track.id,
                    status.join(", ")
                );
            }
        }
        Commands::Favorite { track_id } => {
            let track = cli.set_track_favorite(track_id, true).await?;
            println!("Favorited: {} (ID: {})", track.name, track.id);
        }
        Commands::Unfavorite { track_id } => {
            let track = cli.set_track_favorite(track_id, false).await?;
            println!("Removed from favorites: {} (ID: {})", track.name, track.id);
        }
        Commands::Blacklist { track_id } => {
            let track = cli.set_track_blacklisted(track_id, true).await?;
            println!("Blacklisted: {} (ID: {})", track.name, track.id);
        }
        Commands::Unblacklist { track_id } => {
            let track = cli.set_track_blacklisted(track_id, false).await?;
            println!("Restored: {} (ID: {})", track.name, track.id);
        }
        Commands::Tui => {
            tui::run(event_bus).await?;
        }
    }

    Ok(())
}
