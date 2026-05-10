use rodio::Source;
use rodio::{OutputStream, Sink};
use std::path::Path;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔍 Audio Debug Tool");
    println!("==================");

    // Test 1: Basic audio output
    println!("\n1. Testing basic tone output...");
    {
        let (_stream, stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&stream_handle)?;

        let source = rodio::source::SineWave::new(440.0).take_duration(Duration::from_secs(1));
        sink.append(source);
        sink.set_volume(0.5);

        println!("   Playing 440Hz tone for 1 second...");
        sink.play();

        while !sink.empty() {
            std::thread::sleep(Duration::from_millis(50));
        }
        println!("   Tone playback completed.");
    }

    // Test 2: Check for audio files
    println!("\n2. Checking for audio files...");
    let test_files = [
        "/System/Library/Sounds/Ping.aiff",
        "/System/Library/Sounds/Submarine.aiff",
        "/System/Library/Sounds/Tink.aiff",
    ];

    let mut found_file = None;
    for file_path in &test_files {
        if Path::new(file_path).exists() {
            found_file = Some(file_path);
            break;
        }
    }

    if let Some(file_path) = found_file {
        println!("   Found system sound file: {}", file_path);

        // Test 3: Try to play system sound file
        println!("\n3. Testing system sound file playback...");
        println!("   Note: System sounds may use unsupported formats");
        {
            let (_stream, stream_handle) = OutputStream::try_default()?;
            let sink = Sink::try_new(&stream_handle)?;

            match rodio::Decoder::new(std::io::BufReader::new(std::fs::File::open(file_path)?)) {
                Ok(source) => {
                    sink.append(source);
                    sink.set_volume(0.5);
                    println!("   Playing system sound...");
                    sink.play();

                    while !sink.empty() {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    println!("   System sound playback completed.");
                }
                Err(e) => {
                    println!("   Cannot play system sound (unsupported format): {}", e);
                }
            }
        }
    } else {
        println!("   No system sound files found.");
    }

    // Test 4: Try to get output device info
    println!("\n4. Testing audio device access...");
    match OutputStream::try_default() {
        Ok((stream, _)) => {
            println!("   Successfully created output stream");
            drop(stream); // Drop to release the device
            println!("   Stream released");
        }
        Err(e) => {
            println!("   Failed to create output stream: {}", e);
        }
    }

    println!("\n✅ Audio debug completed.");
    Ok(())
}
