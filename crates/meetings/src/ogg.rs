//! Just enough Ogg for the meeting file: whole packets only, one logical stream.
//!
//! An Ogg page carries its own length and CRC, which is what makes the file crash-safe: a page
//! cut short by a crash fails the check and everything before it is intact.

const CAPTURE_PATTERN: &[u8; 4] = b"OggS";
const HEADER_LEN: usize = 27;
const CRC_OFFSET: usize = 22;
/// Stays inside one page's 255-entry lacing table.
pub const MAX_SEGMENTS: usize = 255;
const SEGMENT_LEN: usize = 255;

const FLAG_BOS: u8 = 0x02;
const FLAG_EOS: u8 = 0x04;

/// Any value works for a single logical stream.
const SERIAL: u32 = 0x5746_4d54;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    First,
    Middle,
    Last,
}

/// Lacing entries one packet takes up.
pub fn segments_for(packet_len: usize) -> usize {
    packet_len / SEGMENT_LEN + 1
}

/// Serialises one page holding `packets`, which must fit in [`MAX_SEGMENTS`] lacing entries.
pub fn page(kind: PageKind, sequence: u32, granule: u64, packets: &[Vec<u8>]) -> Vec<u8> {
    let mut lacing = Vec::new();
    for packet in packets {
        lacing.resize(lacing.len() + packet.len() / SEGMENT_LEN, 255);
        lacing.push((packet.len() % SEGMENT_LEN) as u8);
    }
    debug_assert!(lacing.len() <= MAX_SEGMENTS);

    let flags = match kind {
        PageKind::First => FLAG_BOS,
        PageKind::Middle => 0,
        PageKind::Last => FLAG_EOS,
    };
    let mut page = Vec::with_capacity(HEADER_LEN + lacing.len());
    page.extend_from_slice(CAPTURE_PATTERN);
    page.push(0); // version
    page.push(flags);
    page.extend_from_slice(&granule.to_le_bytes());
    page.extend_from_slice(&SERIAL.to_le_bytes());
    page.extend_from_slice(&sequence.to_le_bytes());
    page.extend_from_slice(&[0; 4]); // CRC, filled in below
    page.push(lacing.len() as u8);
    page.extend_from_slice(&lacing);
    for packet in packets {
        page.extend_from_slice(packet);
    }
    let crc = crc(&page);
    page[CRC_OFFSET..CRC_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());
    page
}

/// A complete, checksummed page found in a file.
#[derive(Debug)]
pub struct Page<'a> {
    pub sequence: u32,
    pub granule: u64,
    pub end_of_stream: bool,
    /// Offset of the first byte after this page.
    pub end: usize,
    pub packets: Vec<&'a [u8]>,
}

/// Every intact page from the start of `bytes`, stopping at the first one that is cut short,
/// damaged or out of sequence.
pub fn pages(bytes: &[u8]) -> Vec<Page<'_>> {
    let mut pages = Vec::new();
    let mut offset = 0;
    while let Some(page) = parse_page(bytes, offset) {
        if page.sequence as usize != pages.len() {
            break;
        }
        offset = page.end;
        pages.push(page);
    }
    pages
}

fn parse_page(bytes: &[u8], start: usize) -> Option<Page<'_>> {
    let header = bytes.get(start..start + HEADER_LEN)?;
    if &header[..4] != CAPTURE_PATTERN {
        return None;
    }
    let lacing_len = usize::from(header[26]);
    let lacing = bytes.get(start + HEADER_LEN..start + HEADER_LEN + lacing_len)?;
    let body_start = start + HEADER_LEN + lacing_len;
    let body_len: usize = lacing.iter().map(|&l| usize::from(l)).sum();
    let end = body_start + body_len;
    let raw = bytes.get(start..end)?;

    let stored = u32::from_le_bytes(raw[CRC_OFFSET..CRC_OFFSET + 4].try_into().ok()?);
    let mut zeroed = raw.to_vec();
    zeroed[CRC_OFFSET..CRC_OFFSET + 4].fill(0);
    if crc(&zeroed) != stored {
        return None;
    }

    let mut packets = Vec::new();
    let (mut packet_start, mut cursor) = (body_start, body_start);
    for &entry in lacing {
        cursor += usize::from(entry);
        if entry < 255 {
            packets.push(&bytes[packet_start..cursor]);
            packet_start = cursor;
        }
    }
    Some(Page {
        sequence: u32::from_le_bytes(header[18..22].try_into().ok()?),
        granule: u64::from_le_bytes(header[6..14].try_into().ok()?),
        end_of_stream: header[5] & FLAG_EOS != 0,
        end,
        packets,
    })
}

/// Ogg's CRC-32: polynomial 0x04c11db7, MSB first, no pre- or post-inversion.
fn crc(bytes: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in bytes {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}
