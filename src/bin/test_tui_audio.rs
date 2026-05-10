use musictui2::audio::AudioPlayer;
use musictui2::models::Track;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎵 TUI Audio Test");
    println!("================");

    let mut audio_player = AudioPlayer::new()?;

    // Create a mock track
    let mock_track = Track {
        id: 1,
        repository_id: 1,
        path: "test.mp3".to_string(),
        name: "Test Track".to_string(),
        format: "mp3".to_string(),
        size: 1024 * 1024, // 1MB
        duration: Some(std::time::Duration::from_secs(30)),
        url: "https://example.com/test.mp3".to_string(),
        local_path: None,
        downloaded: false,
        discovered_at: chrono::Utc::now(),
    };

    println!("\n1. Testing track loading without download...");
    match audio_player.load_track(mock_track.clone()).await {
        Ok(_) => {
            println!("   Track loaded successfully");
            println!("   Playback state: {:?}", audio_player.get_playback_state());
            println!(
                "   Current track: {:?}",
                audio_player.get_current_track().map(|t| t.name.as_str())
            );
        }
        Err(e) => {
            println!("   Failed to load track: {}", e);
            println!("   This is expected since track is not downloaded");
        }
    }

    // Test with a mock downloaded track (using a system sound)
    let mut downloaded_track = mock_track.clone();
    downloaded_track.local_path = Some(PathBuf::from("/System/Library/Sounds/Ping.aiff"));
    downloaded_track.downloaded = true;

    println!("\n2. Testing track loading with local file (system sound)...");
    match audio_player.load_track(downloaded_track.clone()).await {
        Ok(_) => {
            println!("   Track loaded successfully");
            println!("   Playback state: {:?}", audio_player.get_playback_state());

            println!("\n3. Testing playback...");
            match audio_player.play() {
                Ok(_) => {
                    println!("   Playback started");
                    println!("   Waiting for playback to complete...");

                    // Wait for playback to finish
                    let mut attempts = 0;
                    while audio_player.is_playing() && attempts < 60 {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        attempts += 1;
                    }

                    if audio_player.is_playing() {
                        println!("   Playback still active after {} seconds", attempts);
                    } else {
                        println!("   Playback completed");
                    }
                }
                Err(e) => {
                    println!("   Failed to play: {}", e);
                }
            }

            println!("\n4. Testing pause/resume...");
            match audio_player.pause() {
                Ok(_) => {
                    println!("   Paused successfully");
                    std::thread::sleep(std::time::Duration::from_secs(1));

                    match audio_player.play() {
                        Ok(_) => {
                            println!("   Resumed successfully");
                            std::thread::sleep(std::time::Duration::from_secs(1));
                        }
                        Err(e) => {
                            println!("   Failed to resume: {}", e);
                        }
                    }
                }
                Err(e) => {
                    println!("   Failed to pause: {}", e);
                }
            }

            println!("\n5. Testing stop...");
            match audio_player.stop() {
                Ok(_) => {
                    println!("   Stopped successfully");
                    println!("   Playback state: {:?}", audio_player.get_playback_state());
                }
                Err(e) => {
                    println!("   Failed to stop: {}", e);
                }
            }
        }
        Err(e) => {
            println!("   Failed to load track with local file: {}", e);
        }
    }

    println!("\n✅ TUI audio test completed.");
    Ok(())
}
