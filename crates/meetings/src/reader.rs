//! Reading meeting files back, whole or cut short by a crash.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use opus::MSDecoder;
use wf_core::audio::{AudioClip, FRAME_SAMPLES, SAMPLE_RATE_HZ};

use crate::Error;
use crate::ogg;
use crate::ogg::PageKind;
use crate::writer::{CHANNEL_MAPPING, GRANULE_PER_SAMPLE, GRANULE_RATE_HZ, STREAMS};

const HEADER_PAGES: usize = 2;
const PRE_SKIP_OFFSET: usize = 10;

/// A meeting's two channels, on the same timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingAudio {
    /// The microphone.
    pub you: AudioClip,
    /// System audio.
    pub others: AudioClip,
}

/// Decodes everything up to the end of the file or the first damaged page.
pub fn read_meeting(path: &Path) -> Result<MeetingAudio, Error> {
    let bytes = fs::read(path)?;
    let pages = ogg::pages(&bytes);
    let pre_skip = pre_skip(&pages).ok_or_else(|| Error::NotAMeeting(path.to_owned()))?;

    let mut decoder = MSDecoder::new(SAMPLE_RATE_HZ, STREAMS, 0, &CHANNEL_MAPPING)?;
    let channels = usize::from(STREAMS);
    let mut pcm = vec![0.0; FRAME_SAMPLES * channels];
    let (mut you, mut others) = (Vec::new(), Vec::new());
    for page in &pages[HEADER_PAGES..] {
        for packet in &page.packets {
            let decoded = decoder.decode_float(packet, &mut pcm, false)?;
            for frame in pcm[..decoded * channels].chunks_exact(channels) {
                you.push(frame[0]);
                others.push(frame[1]);
            }
        }
    }

    let skip = (pre_skip / GRANULE_PER_SAMPLE) as usize;
    let end = pages.last().map_or(0, |page| page.granule);
    let len = (end.saturating_sub(pre_skip) / GRANULE_PER_SAMPLE) as usize;
    let trim = |mut samples: Vec<f32>| {
        samples.drain(..skip.min(samples.len()));
        samples.truncate(len);
        AudioClip::new(samples)
    };
    Ok(MeetingAudio {
        you: trim(you),
        others: trim(others),
    })
}

/// Turns a recording a crash left behind into a finished file: drops the torn tail and marks the
/// end of the stream. Safe on an intact file, and safe to interrupt: the original stays in place
/// until the repaired copy has fully replaced it. Returns the recovered length.
pub fn repair_meeting(path: &Path) -> Result<Duration, Error> {
    let bytes = fs::read(path)?;
    let pages = ogg::pages(&bytes);
    let pre_skip = pre_skip(&pages).ok_or_else(|| Error::NotAMeeting(path.to_owned()))?;
    let last = pages.last().expect("a meeting has header pages");
    let audio = last.granule.saturating_sub(pre_skip);
    let duration = Duration::from_secs_f64(audio as f64 / f64::from(GRANULE_RATE_HZ));
    if last.end_of_stream && last.end == bytes.len() {
        return Ok(duration);
    }

    let mut repaired = bytes[..last.end].to_vec();
    if pages.len() == HEADER_PAGES {
        repaired.extend(ogg::page(PageKind::Last, last.sequence + 1, 0, &[]));
    } else {
        let packets: Vec<Vec<u8>> = last.packets.iter().map(|p| p.to_vec()).collect();
        let page = ogg::page(PageKind::Last, last.sequence, last.granule, &packets);
        repaired.truncate(last.end - page.len());
        repaired.extend(page);
    }

    let temporary = path.with_extension("repairing");
    let mut file = File::create(&temporary)?;
    file.write_all(&repaired)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    Ok(duration)
}

/// The OpusHead pre-skip, if the file starts with this format's two header pages.
fn pre_skip(pages: &[ogg::Page<'_>]) -> Option<u64> {
    let head = pages.first()?.packets.first()?;
    let tags = pages.get(1)?.packets.first()?;
    if !head.starts_with(b"OpusHead") || !tags.starts_with(b"OpusTags") {
        return None;
    }
    let bytes = head.get(PRE_SKIP_OFFSET..PRE_SKIP_OFFSET + 2)?;
    Some(u64::from(u16::from_le_bytes(bytes.try_into().ok()?)))
}
