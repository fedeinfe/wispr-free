//! The meeting file: Ogg Opus, written a page at a time so a crash costs at most the current
//! page. L = You, R = Others.
//!
//! The channels are two independent mono Opus streams (RFC 7845 mapping family 1), not one
//! coupled stereo stream: they are different audio that only happen to share a timeline, and
//! joint stereo would bleed one into the other. Players still see ordinary stereo.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use opus::{Application, Bitrate, MSEncoder};
use wf_core::audio::{FRAME_SAMPLES, SAMPLE_RATE_HZ};

use crate::Error;
use crate::ogg::{self, MAX_SEGMENTS, PageKind};

/// Opus granule positions always count 48 kHz samples.
pub(crate) const GRANULE_RATE_HZ: u32 = 48_000;
pub(crate) const GRANULE_PER_SAMPLE: u64 = (GRANULE_RATE_HZ / SAMPLE_RATE_HZ) as u64;
pub(crate) const STREAMS: u8 = 2;
pub(crate) const CHANNEL_MAPPING: [u8; 2] = [0, 1];

/// Audio goes to disk this often, so this is the most a crash can lose.
const PAGE_FRAMES: usize = 25;
const BITRATE_PER_CHANNEL: i32 = 24_000;
const MAX_PACKET_LEN: usize = 4_000;

/// Writes a meeting incrementally. Dropping it without [`finish`](Self::finish), or crashing,
/// leaves a file that [`read_meeting`](crate::read_meeting) and
/// [`repair_meeting`](crate::repair_meeting) can use up to the last complete page.
pub struct MeetingWriter {
    file: File,
    encoder: MSEncoder,
    pre_skip: u64,
    next_sequence: u32,
    frames: u64,
    pending: Vec<Vec<u8>>,
    pending_segments: usize,
    interleaved: Vec<f32>,
}

impl MeetingWriter {
    /// Creates the file and makes its headers durable. Fails if `path` already exists: a
    /// recording is never overwritten.
    pub fn create(path: &Path) -> Result<Self, Error> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut encoder = MSEncoder::new(
            SAMPLE_RATE_HZ,
            STREAMS,
            0,
            &CHANNEL_MAPPING,
            Application::Voip,
        )?;
        encoder.set_bitrate(Bitrate::Bits(BITRATE_PER_CHANNEL * i32::from(STREAMS)))?;
        let lookahead = u64::try_from(encoder.get_lookahead()?).unwrap_or(0);
        let pre_skip = lookahead * GRANULE_PER_SAMPLE;

        let mut writer = Self {
            file,
            encoder,
            pre_skip,
            next_sequence: 0,
            frames: 0,
            pending: Vec::new(),
            pending_segments: 0,
            interleaved: vec![0.0; FRAME_SAMPLES * usize::from(STREAMS)],
        };
        writer.write_page(PageKind::First, 0, &[opus_head(pre_skip as u16)])?;
        writer.write_page(PageKind::Middle, 0, &[opus_tags()])?;
        Ok(writer)
    }

    /// Appends one 40 ms frame per channel, each exactly [`FRAME_SAMPLES`] samples.
    pub fn write_frame(&mut self, you: &[f32], others: &[f32]) -> Result<(), Error> {
        assert_eq!(you.len(), FRAME_SAMPLES);
        assert_eq!(others.len(), FRAME_SAMPLES);
        let channels = usize::from(STREAMS);
        for (n, frame) in self.interleaved.chunks_exact_mut(channels).enumerate() {
            frame[0] = you[n];
            frame[1] = others[n];
        }
        self.encode()?;
        self.frames += 1;
        if self.pending.len() >= PAGE_FRAMES {
            self.flush_pending()?;
        }
        Ok(())
    }

    /// Writes what is left and marks the end of the stream. Returns the meeting's length.
    pub fn finish(mut self) -> Result<Duration, Error> {
        if self.frames > 0 {
            // The codec delays everything by its lookahead. One more frame pushes the tail of
            // the last real frame out; the granule position below cuts the padding off again.
            self.interleaved.fill(0.0);
            self.encode()?;
        }
        let end = self.end_granule();
        let granule = if self.frames > 0 {
            end + self.pre_skip
        } else {
            0
        };
        let packets = std::mem::take(&mut self.pending);
        self.write_page(PageKind::Last, granule, &packets)?;
        Ok(Duration::from_secs_f64(
            end as f64 / f64::from(GRANULE_RATE_HZ),
        ))
    }

    fn encode(&mut self) -> Result<(), Error> {
        let packet = self
            .encoder
            .encode_vec_float(&self.interleaved, MAX_PACKET_LEN)?;
        let segments = ogg::segments_for(packet.len());
        if self.pending_segments + segments > MAX_SEGMENTS {
            self.flush_pending()?;
        }
        self.pending_segments += segments;
        self.pending.push(packet);
        Ok(())
    }

    fn end_granule(&self) -> u64 {
        self.frames * FRAME_SAMPLES as u64 * GRANULE_PER_SAMPLE
    }

    fn flush_pending(&mut self) -> Result<(), Error> {
        let packets = std::mem::take(&mut self.pending);
        self.pending_segments = 0;
        self.write_page(PageKind::Middle, self.end_granule(), &packets)
    }

    fn write_page(
        &mut self,
        kind: PageKind,
        granule: u64,
        packets: &[Vec<u8>],
    ) -> Result<(), Error> {
        let page = ogg::page(kind, self.next_sequence, granule, packets);
        self.next_sequence += 1;
        self.file.write_all(&page)?;
        self.file.sync_data()?;
        Ok(())
    }
}

fn opus_head(pre_skip: u16) -> Vec<u8> {
    let mut head = Vec::with_capacity(21);
    head.extend_from_slice(b"OpusHead");
    head.push(1); // version
    head.push(STREAMS); // output channels
    head.extend_from_slice(&pre_skip.to_le_bytes());
    head.extend_from_slice(&SAMPLE_RATE_HZ.to_le_bytes());
    head.extend_from_slice(&0i16.to_le_bytes()); // output gain
    head.push(1); // channel mapping family: Vorbis order, here L then R
    head.push(STREAMS);
    head.push(0); // coupled streams
    head.extend_from_slice(&CHANNEL_MAPPING);
    head
}

fn opus_tags() -> Vec<u8> {
    let vendor = concat!("wispr-free ", env!("CARGO_PKG_VERSION"));
    let mut tags = Vec::new();
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    tags.extend_from_slice(vendor.as_bytes());
    tags.extend_from_slice(&0u32.to_le_bytes()); // user comments
    tags
}
