use musictui2::audio::AudioPlayer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing audio output...");

    let mut audio_player = AudioPlayer::new()?;

    // Test audio with a tone
    println!("Playing a 440Hz tone for 1 second...");
    audio_player.test_audio()?;

    println!("Audio test completed!");
    Ok(())
}