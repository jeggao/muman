//! The output gain an Ogg Opus file's header holds, read and set in place.
//!
//! RFC 7845 puts the Opus identification header alone on the stream's
//! first page, and in it a gain in 1/256 dB that every decoder applies
//! to all it decodes: a song made quieter there is quieter in every
//! player, with not a packet of its audio touched. Setting it rewrites
//! two bytes of that page and the page's CRC, the Ogg CRC-32 (polynomial
//! 0x04C11DB7, no reflection, starting at 0) over the page with its CRC
//! field zeroed; nothing else in the file moves.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use anyhow::{Context, Result, bail};

/// Where a page's CRC sits, and its segment count.
const CRC_AT: usize = 22;
const SEGMENTS_AT: usize = 26;
const PAGE_HEADER: usize = 27;
/// Where the gain sits in the identification header.
const GAIN_AT: usize = 16;
const BEGINS_STREAM: u8 = 0x02;

const CRC_TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        #[allow(clippy::cast_possible_truncation)]
        let mut r = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            r = if r & 0x8000_0000 == 0 {
                r << 1
            } else {
                (r << 1) ^ 0x04C1_1DB7
            };
            bit += 1;
        }
        table[i] = r;
        i += 1;
    }
    table
};

fn crc(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |crc, &b| {
        (crc << 8) ^ CRC_TABLE[(((crc >> 24) as u8) ^ b) as usize]
    })
}

/// The first page of `bytes`, if it holds an Opus identification header
/// alone: the page's length and where the header's packet begins.
fn id_page(bytes: &[u8]) -> Option<(usize, usize)> {
    if bytes.get(..4)? != b"OggS" || bytes.get(5)? & BEGINS_STREAM == 0 {
        return None;
    }
    let segments = usize::from(*bytes.get(SEGMENTS_AT)?);
    let table = bytes.get(PAGE_HEADER..PAGE_HEADER + segments)?;
    // One packet, ended on this page: no lacing value of 255 but before the last.
    if table.last().is_none_or(|&l| l == 255) {
        return None;
    }
    let body = PAGE_HEADER + segments;
    let length: usize = table.iter().map(|&l| usize::from(l)).sum();
    let packet = bytes.get(body..body + length)?;
    (packet.len() > GAIN_AT + 1 && packet.starts_with(b"OpusHead")).then_some((body + length, body))
}

/// The output gain of the Ogg Opus file `bytes` begin, in 1/256 dB.
#[must_use]
pub fn output_gain(bytes: &[u8]) -> Option<i16> {
    let (_, packet) = id_page(bytes)?;
    let at = packet + GAIN_AT;
    Some(i16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

/// `gain` written as the output gain of the Ogg Opus file `file`.
pub fn set_output_gain(file: &mut File, gain: i16) -> Result<()> {
    let mut head = Vec::new();
    file.seek(SeekFrom::Start(0))?;
    // The longest first page: its header, 255 lacing values and their bodies.
    (&mut *file)
        .take((PAGE_HEADER + 255 + 255 * 255) as u64)
        .read_to_end(&mut head)
        .context("reading the Opus header")?;
    let Some((length, packet)) = id_page(&head) else {
        bail!("no Opus header alone on the first page");
    };
    let mut page = head[..length].to_vec();
    page[packet + GAIN_AT..packet + GAIN_AT + 2].copy_from_slice(&gain.to_le_bytes());
    page[CRC_AT..CRC_AT + 4].fill(0);
    let sum = crc(&page);
    page[CRC_AT..CRC_AT + 4].copy_from_slice(&sum.to_le_bytes());
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&page).context("writing the Opus header")?;
    file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use lofty::file::AudioFile;

    use super::*;
    use crate::testing::SILENCE_OPUS;

    #[test]
    fn the_fixtures_first_page_sums_to_its_crc() {
        let (length, _) = id_page(SILENCE_OPUS).unwrap();
        let mut page = SILENCE_OPUS[..length].to_vec();
        let stored = u32::from_le_bytes(page[CRC_AT..CRC_AT + 4].try_into().unwrap());
        page[CRC_AT..CRC_AT + 4].fill(0);
        assert_eq!(crc(&page), stored);
        assert_eq!(output_gain(SILENCE_OPUS), Some(0));
        assert_eq!(output_gain(crate::testing::SILENCE_VORBIS), None);
    }

    #[test]
    fn a_gain_set_reads_back_and_the_file_still_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.opus");
        std::fs::write(&path, SILENCE_OPUS).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        set_output_gain(&mut file, -1567).unwrap();
        drop(file);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(output_gain(&bytes), Some(-1567));
        assert_eq!(bytes.len(), SILENCE_OPUS.len());
        let (length, _) = id_page(&bytes).unwrap();
        let mut page = bytes[..length].to_vec();
        let stored = u32::from_le_bytes(page[CRC_AT..CRC_AT + 4].try_into().unwrap());
        page[CRC_AT..CRC_AT + 4].fill(0);
        assert_eq!(crc(&page), stored);
        let mut reopened = File::open(&path).unwrap();
        lofty::ogg::OpusFile::read_from(&mut reopened, lofty::config::ParseOptions::new()).unwrap();
        let mut vorbis = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        std::fs::write(&path, crate::testing::SILENCE_VORBIS).unwrap();
        assert!(set_output_gain(&mut vorbis, 1).is_err());
    }
}
