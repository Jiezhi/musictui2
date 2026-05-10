use crate::cache::{GrowingFileReader, StreamingCacheState};
use crate::github::GitHubScanner;
use crate::models::{PlaybackState, Track};
use rodio::{source::SineWave, Decoder, OutputStream, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub type StreamingDecoder = Decoder<GrowingFileReader>;

pub struct AudioPlayer {
    current_track: Option<Track>,
    playback_state: PlaybackState,
    sink: Option<Sink>,
    volume: f32,
    #[allow(dead_code)]
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
    pub fn with_github_scanner(
        scanner: Arc<GitHubScanner>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            current_track: None,
            playback_state: PlaybackState::Stopped,
            sink: None,
            volume: 1.0,
            github_scanner: Some(scanner),
            output_stream: None,
        })
    }

    #[allow(dead_code)]
    pub async fn load_track(&mut self, track: Track) -> Result<(), Box<dyn std::error::Error>> {
        if track.is_playable() {
            return self.load_local_track(track);
        }

        // If track is not downloaded, try to download it
        if let Some(scanner) = &self.github_scanner {
            match scanner.download_track(&track).await {
                Ok(local_path) => {
                    let mut downloaded_track = track;
                    downloaded_track.local_path = Some(local_path);
                    downloaded_track.downloaded = true;
                    return self.load_local_track(downloaded_track);
                }
                Err(e) => {
                    return Err(format!("Failed to download track: {}", e).into());
                }
            }
        }

        // If track is not downloaded and no scanner available
        Err("Track not downloaded or file not found. Please download the track first.".into())
    }

    pub fn load_local_track(&mut self, track: Track) -> Result<(), Box<dyn std::error::Error>> {
        self.stop()?;

        let local_path = track
            .local_path
            .as_ref()
            .ok_or("Track has no local file path")?;

        if !local_path.exists() {
            return Err(format!("Track file not found: {}", local_path.display()).into());
        }

        let (stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;
        let file = File::open(local_path)?;
        let source = Decoder::new(BufReader::new(file))?;

        sink.set_volume(self.volume);
        sink.append(source);
        sink.play();

        self.output_stream = Some(stream);
        self.current_track = Some(track);
        self.playback_state = PlaybackState::Playing;
        self.sink = Some(sink);

        Ok(())
    }

    pub fn load_streaming_track(
        &mut self,
        track: Track,
        source: StreamingDecoder,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.stop()?;

        let (stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;

        sink.set_volume(self.volume);
        sink.append(source);
        sink.play();

        self.output_stream = Some(stream);
        self.current_track = Some(track);
        self.playback_state = PlaybackState::Playing;
        self.sink = Some(sink);

        Ok(())
    }

    #[allow(dead_code)]
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
        self.current_track.as_ref().and_then(|track| track.duration)
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

pub fn prepare_streaming_decoder(
    cache_path: PathBuf,
    state: StreamingCacheState,
    format: String,
    initial_buffer_bytes: u64,
) -> Result<StreamingDecoder, String> {
    state.wait_for_bytes(initial_buffer_bytes)?;
    let reader =
        GrowingFileReader::open(&cache_path, state.clone()).map_err(|err| err.to_string())?;

    match decode_streaming_reader(reader, &format) {
        Ok(decoder) => Ok(decoder),
        Err(_first_error) if !state.is_complete() => {
            state.wait_until_complete().map_err(|err| err.to_string())?;
            let reader =
                GrowingFileReader::open(cache_path, state).map_err(|err| err.to_string())?;
            decode_streaming_reader(reader, &format).map_err(|err| err.to_string())
        }
        Err(err) => Err(format!("{err}")),
    }
}

fn decode_streaming_reader(
    reader: GrowingFileReader,
    format: &str,
) -> Result<StreamingDecoder, rodio::decoder::DecoderError> {
    match format {
        "flac" => Decoder::new_flac(reader),
        "mp3" => Decoder::new_mp3(reader),
        "ogg" => Decoder::new_vorbis(reader),
        "wav" => Decoder::new_wav(reader),
        _ => Decoder::new(reader),
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
        use std::io::Write;
        use tempfile::NamedTempFile;

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
