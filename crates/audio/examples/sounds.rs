//! Plays the pill's cues in turn: `cargo run -p wf-audio --example sounds`.

use std::time::Duration;

use wf_audio::sounds::play_blocking;
use wf_core::pill::Sound;

fn main() -> Result<(), wf_audio::Error> {
    for sound in [Sound::Start, Sound::Stop, Sound::Lock, Sound::Error] {
        println!("{sound:?}");
        play_blocking(sound)?;
        std::thread::sleep(Duration::from_millis(400));
    }
    Ok(())
}
