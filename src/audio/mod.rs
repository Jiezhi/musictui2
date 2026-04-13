use std::time::Duration;
use rodio::{Decoder, OutputStream, Sink, source::SineWave, Source};
use crate::models::{Track, PlaybackState};
use crate::github::GitHubScanner;
use std::sync::Arc;

pub struct AudioPlayer {
    current_track: Option<Track>,
    playback_state: PlaybackState,
    sink: Option<Sink>,
    volume: f32,
    github_scanner: Option<Arc<GitHubScanner>>,
    output_stream: Option<OutputStream>,
}

impl AudioPlayer {
    #[allow(dead_code)]
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            current_track: None,
            playback_state: PlaybackState::Stopped,
            sink: None,
            volume: 1.0,
            github_scanner: None,
            output_stream: None,
        })
    }

    #[allow(dead_code)]
    pub fn with_github_scanner(scanner: Arc<GitHubScanner>) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            current_track: None,
            playback_state: PlaybackState::Stopped,
            sink: None,
            volume: 1.0,
            github_scanner: Some(scanner),
            output_stream: None,
        })
    }

    pub async fn load_track(&mut self, track: Track) -> Result<(), Box<dyn std::error::Error>> {
        // Stop current playback
        self.stop()?;

        // Create new output stream and sink
        let (stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;

        // Store the output stream to keep it alive
        self.output_stream = Some(stream);

        // Check if track is downloaded
        if let Some(local_path) = &track.local_path {
            if local_path.exists() {
                // Load audio file
                let file = std::fs::File::open(local_path)?;
                let source = Decoder::new(std::io::BufReader::new(file))?;

                // Set volume
                sink.set_volume(self.volume);

                // Play the track
                sink.append(source);

                self.current_track = Some(track);
                self.playback_state = PlaybackState::Playing;
                self.sink = Some(sink);

                return Ok(());
            }
        }

        // If track is not downloaded, try to download it
        if let Some(scanner) = &self.github_scanner {
            match scanner.download_track(&track).await {
                Ok(local_path) => {
                    // Load audio file
                    let file = std::fs::File::open(&local_path)?;
                    let source = Decoder::new(std::io::BufReader::new(file))?;

                    // Set volume
                    sink.set_volume(self.volume);

                    // Play the track
                    sink.append(source);

                    self.current_track = Some(track);
                    self.playback_state = PlaybackState::Playing;
                    self.sink = Some(sink);

                    return Ok(());
                }
                Err(e) => {
                    return Err(format!("Failed to download track: {}", e).into());
                }
            }
        }

        // If track is not downloaded and no scanner available
        Err("Track not downloaded or file not found. Please download the track first.".into())
    }

    pub fn play(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(sink) = &mut self.sink {
            if sink.empty() {
                self.stop()?;
                return Err("No track loaded".into());
            }

            sink.play();
            self.playback_state = PlaybackState::Playing;
            Ok(())
        } else {
            Err("No track loaded".into())
        }
    }

    pub fn pause(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(sink) = &mut self.sink {
            sink.pause();
            self.playback_state = PlaybackState::Paused;
            Ok(())
        } else {
            Err("No track loaded".into())
        }
    }

    pub fn stop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(sink) = self.sink.take() {
            sink.stop();
            self.playback_state = PlaybackState::Stopped;
        }

        // Drop the output stream to release the audio device
        self.output_stream = None;

        self.current_track = None;
        Ok(())
    }

    pub fn set_volume(&mut self, volume: f32) -> Result<(), Box<dyn std::error::Error>> {
        // Clamp volume between 0.0 and 1.0
        let volume = volume.clamp(0.0, 1.0);

        if let Some(sink) = &mut self.sink {
            sink.set_volume(volume);
        }

        self.volume = volume;
        Ok(())
    }

    pub fn get_volume(&self) -> f32 {
        self.volume
    }

    pub fn get_current_track(&self) -> Option<&Track> {
        self.current_track.as_ref()
    }

    pub fn get_playback_state(&self) -> &PlaybackState {
        &self.playback_state
    }

    pub fn is_playing(&self) -> bool {
        matches!(self.playback_state, PlaybackState::Playing)
    }

    #[allow(dead_code)]
    pub fn is_paused(&self) -> bool {
        matches!(self.playback_state, PlaybackState::Paused)
    }

    #[allow(dead_code)]
    pub fn get_progress(&self) -> Option<Duration> {
        // TODO: Implement progress tracking using rodio's position API
        // For now, return None as the current version doesn't support this
        None
    }

    #[allow(dead_code)]
    pub fn get_duration(&self) -> Option<Duration> {
        self.current_track
            .as_ref()
            .and_then(|track| track.duration)
    }

    #[allow(dead_code)]
    pub fn get_sink(&mut self) -> Option<&mut Sink> {
        self.sink.as_mut()
    }

    /// Test audio output with a tone
    #[allow(dead_code)]
    pub fn test_audio(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Create a new output stream for testing
        let (stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;

        // Play a test tone
        let source = SineWave::new(440.0).take_duration(Duration::from_secs(1));
        sink.append(source);
        sink.set_volume(self.volume);

        // Start playback
        sink.play();

        // Wait for playback to finish
        while !sink.empty() {
            std::thread::sleep(Duration::from_millis(50));
        }

        // Clean up
        drop(sink);
        drop(stream);

        Ok(())
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl std::fmt::Debug for AudioPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioPlayer")
            .field("current_track", &"...")
            .field("playback_state", &self.playback_state)
            .field("volume", &self.volume)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Track;
    
    #[test]
    fn test_audio_player_creation() {
        let player = AudioPlayer::new();
        assert!(player.is_ok());
        let player = player.unwrap();
        assert!(player.get_current_track().is_none());
        assert!(!player.is_playing());
        assert!(!player.is_paused());
    }

    #[test]
    fn test_volume_clamping() {
        let mut player = AudioPlayer::new().unwrap();

        // Test volume clamping
        player.set_volume(1.5).unwrap();
        assert_eq!(player.get_volume(), 1.0);

        player.set_volume(-0.5).unwrap();
        assert_eq!(player.get_volume(), 0.0);

        player.set_volume(0.5).unwrap();
        assert_eq!(player.get_volume(), 0.5);
    }

    #[test]
    fn test_track_playable_status() {
        use tempfile::NamedTempFile;
        use std::io::Write;

        // Create a temporary file
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "test audio content").unwrap();
        let temp_path = temp_file.path().to_path_buf();

        let mut track = Track {
            id: 1,
            repository_id: 1,
            path: "/test/track.mp3".to_string(),
            name: "test-track".to_string(),
            format: "mp3".to_string(),
            size: 1024,
            duration: Some(std::time::Duration::from_secs(180)),
            url: "https://example.com/track.mp3".to_string(),
            local_path: None,
            downloaded: false,
            discovered_at: chrono::Utc::now(),
        };

        assert!(!track.is_playable());

        // Mark as downloaded but no local path
        track.downloaded = true;
        assert!(!track.is_playable());

        // Add local path to existing file
        track.local_path = Some(temp_path.clone());
        assert!(track.is_playable());

        // Remove local path
        track.local_path = None;
        assert!(!track.is_playable());

        // Clean up
        drop(temp_file);
    }
}