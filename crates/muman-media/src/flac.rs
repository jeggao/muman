//! The sample count a FLAC's STREAMINFO states, made good in place.
//!
//! STREAMINFO, the first metadata block, states the stream's samples in 36
//! bits, 0 where its writer did not know them: a FLAC encoded into a pipe,
//! which cannot seek back to its header, or an Ogg FLAC, whose header goes
//! out before its audio. ffmpeg's FLAC muxer writes the count only from
//! what its encoder reports once it ends, so a stream copied with `-c copy`
//! keeps the 0 it came with, and players then show no length and cannot
//! seek by time. [`state_samples`] counts them from the stream's last
//! frame, whose header states its first sample, in a stream of variable
//! block sizes, or its number, in one of fixed blocks each STREAMINFO's
//! largest, and its own block size. The last frame is the one at a frame
//! sync, read back from the file's end, whose header's CRC-8 holds and
//! whose CRC-16, over the frame to the end of the file, holds too; a
//! frame is at most 65535 samples of 8 channels of 32 bits, so the file's
//! last [`TAIL`] bytes hold it. Nothing but those 36 bits changes: the
//! MD5 of the audio, which 0 says is unknown alike, is left as it is.

use std::io::{Read, Seek, SeekFrom, Write};

use anyhow::{Context, Result, bail};
use crc::{CRC_8_SMBUS, CRC_16_UMTS, Crc};

/// The bytes before STREAMINFO: `fLaC` and the block's header.
const STREAMINFO_AT: u64 = 8;
const STREAMINFO_LEN: usize = 34;
/// Where the 36-bit sample count begins in STREAMINFO, in the low half of
/// its first byte.
const SAMPLES_AT: usize = 13;
/// The most of the file's end read for its last frame.
const TAIL: u64 = 4 << 20;

/// Set the sample count `file`'s STREAMINFO states, where it states none,
/// from its last frame: the count set, or `None` where it stated one.
///
/// # Errors
///
/// When `file` is no FLAC, its last frame cannot be found, or it cannot
/// be read or written.
pub fn state_samples<F: Read + Write + Seek>(file: &mut F) -> Result<Option<u64>> {
    let mut head = [0; STREAMINFO_AT as usize + STREAMINFO_LEN];
    file.rewind()?;
    file.read_exact(&mut head).context("reading STREAMINFO")?;
    if &head[..4] != b"fLaC" || head[4] & 0x7F != 0 {
        bail!("no FLAC STREAMINFO");
    }
    let info = &head[STREAMINFO_AT as usize..];
    if samples(info) != 0 {
        return Ok(None);
    }
    let block = u64::from(u16::from_be_bytes([info[2], info[3]]));
    let len = file.seek(SeekFrom::End(0))?;
    let from = len.saturating_sub(TAIL).max(head.len() as u64);
    file.seek(SeekFrom::Start(from))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    let Some(total) = (0..tail.len())
        .rev()
        .find_map(|at| frame_end(&tail[at..], block))
    else {
        bail!("its last frame could not be read");
    };
    let mut field = [0; 5];
    field.copy_from_slice(&info[SAMPLES_AT..SAMPLES_AT + 5]);
    field[0] = (field[0] & 0xF0) | ((total >> 32) as u8 & 0x0F);
    field[1..].copy_from_slice(&(total as u32).to_be_bytes());
    file.seek(SeekFrom::Start(STREAMINFO_AT + SAMPLES_AT as u64))?;
    file.write_all(&field)?;
    Ok(Some(total))
}

/// The 36-bit sample count of STREAMINFO `info`.
fn samples(info: &[u8]) -> u64 {
    let low = u32::from_be_bytes([info[14], info[15], info[16], info[17]]);
    (u64::from(info[SAMPLES_AT] & 0x0F) << 32) | u64::from(low)
}

/// The sample after the frame `bytes` begins with, if they hold one
/// frame exactly, its CRCs holding; `block`, a fixed-block stream's
/// block size.
fn frame_end(bytes: &[u8], block: u64) -> Option<u64> {
    let [0xFF, sync, sizes, layout, ..] = *bytes else {
        return None;
    };
    let (size_code, rate_code) = (sizes >> 4, sizes & 0x0F);
    if sync & 0xFE != 0xF8
        || size_code == 0
        || rate_code == 0x0F
        || layout >> 4 > 10
        || (layout >> 1) & 0x07 == 3
        || layout & 1 != 0
    {
        return None;
    }
    let variable = sync & 1 == 1;
    let (number, mut at) = coded(bytes.get(4..)?, if variable { 7 } else { 6 })?;
    let mut take = |n: usize| {
        let field = bytes.get(4 + at..4 + at + n)?;
        at += n;
        Some(field.iter().fold(0u64, |v, &b| (v << 8) | u64::from(b)))
    };
    let size = match size_code {
        1 => 192,
        2..=5 => 576 << (size_code - 2),
        6 => take(1)? + 1,
        7 => take(2)? + 1,
        _ => 256 << (size_code - 8),
    };
    match rate_code {
        12 => _ = take(1)?,
        13 | 14 => _ = take(2)?,
        _ => {}
    }
    let header = 4 + at;
    let (body, stated) = bytes.split_at(bytes.len().checked_sub(2)?);
    if header >= body.len()
        || HEADER_CRC.checksum(&bytes[..header]) != bytes[header]
        || FRAME_CRC.checksum(body) != u16::from_be_bytes([stated[0], stated[1]])
    {
        return None;
    }
    let first = if variable { number } else { number * block };
    Some(first + size)
}

/// The number coded as UTF-8 codes a character, but in up to `most`
/// bytes, and the bytes it takes.
fn coded(bytes: &[u8], most: usize) -> Option<(u64, usize)> {
    let first = *bytes.first()?;
    let len = match first.leading_ones() as usize {
        0 => 1,
        1 => return None,
        n => n,
    };
    if len > most {
        return None;
    }
    let mask = if len == 1 { 0x7F } else { 0x7F >> len };
    let mut value = u64::from(first & mask);
    for &b in bytes.get(1..len)? {
        if b & 0xC0 != 0x80 {
            return None;
        }
        value = (value << 6) | u64::from(b & 0x3F);
    }
    Some((value, len))
}

/// The CRC of a frame header: CRC-8, polynomial 0x07, starting at 0.
static HEADER_CRC: Crc<u8> = Crc::<u8>::new(&CRC_8_SMBUS);

/// The CRC of a frame: CRC-16, polynomial 0x8005, starting at 0.
static FRAME_CRC: Crc<u16> = Crc::<u16>::new(&CRC_16_UMTS);

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A frame of `size` samples at `number`, a frame's number or its
    /// first sample, of one constant subframe.
    fn frame(variable: bool, number: u64, size: u16) -> Vec<u8> {
        let mut f = vec![0xFF, 0xF8 | u8::from(variable), 0x79, 0x08];
        let mut coded = Vec::new();
        if number < 0x80 {
            coded.push(number as u8);
        } else {
            let mut rest = number;
            let mut tail = Vec::new();
            while rest >= 1 << (6 - tail.len()) {
                tail.push(0x80 | (rest & 0x3F) as u8);
                rest >>= 6;
            }
            let n = tail.len() + 1;
            coded.push((0xFF00u16 >> n) as u8 | rest as u8);
            coded.extend(tail.iter().rev());
        }
        f.extend(coded);
        f.extend((size - 1).to_be_bytes());
        f.push(HEADER_CRC.checksum(&f));
        // A constant subframe of 16-bit zeros, padded to a byte.
        f.extend([0x00, 0x00, 0x00]);
        let crc = FRAME_CRC.checksum(&f);
        f.extend(crc.to_be_bytes());
        f
    }

    fn flac(total: u64, frames: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = b"fLaC".to_vec();
        bytes.extend([0x80, 0, 0, 34]);
        let mut info = [0u8; 34];
        info[0..2].copy_from_slice(&4096u16.to_be_bytes());
        info[2..4].copy_from_slice(&4096u16.to_be_bytes());
        info[10..14].copy_from_slice(&[0x0A, 0xC4, 0x40, 0xF0]);
        info[13] |= (total >> 32) as u8;
        info[14..18].copy_from_slice(&(total as u32).to_be_bytes());
        info[18..].fill(0x5A);
        bytes.extend(info);
        for f in frames {
            bytes.extend(f);
        }
        bytes
    }

    #[test]
    fn the_crcs_are_flac_s() {
        assert_eq!(HEADER_CRC.checksum(b"123456789"), 0xF4);
        assert_eq!(FRAME_CRC.checksum(b"123456789"), 0xFEE8);
        assert_eq!(
            HEADER_CRC.checksum(&[0xFF, 0xF8, 0x69, 0x08, 0x00, 0x13]),
            0x64
        );
    }

    #[test]
    fn a_count_of_none_is_read_from_the_last_frame() {
        let frames: Vec<Vec<u8>> = (0..300).map(|n| frame(false, n, 4096)).collect();
        let mut last = frames;
        last.push(frame(false, 300, 1000));
        let mut file = Cursor::new(flac(0, &last));
        assert_eq!(state_samples(&mut file).unwrap(), Some(300 * 4096 + 1000));
        let bytes = file.into_inner();
        assert_eq!(samples(&bytes[8..42]), 300 * 4096 + 1000);
        assert_eq!(bytes[..21], flac(300 * 4096 + 1000, &[])[..21]);
        assert_eq!(bytes[26..42], [0x5A; 16], "the MD5 as it was");
    }

    #[test]
    fn ffmpeg_s_own_flac_is_counted_as_it_states() {
        let mut bytes = crate::testing::SILENCE_FLAC.to_vec();
        let stated = samples(&bytes[8..42]);
        assert!(stated > 0);
        bytes[21] &= 0xF0;
        bytes[22..26].fill(0);
        let mut file = Cursor::new(bytes);
        assert_eq!(state_samples(&mut file).unwrap(), Some(stated));
        assert_eq!(file.into_inner(), crate::testing::SILENCE_FLAC);
    }

    #[test]
    fn a_variable_stream_s_last_frame_states_its_first_sample() {
        let past_32_bits = (1u64 << 33) + 7;
        let frames = [frame(true, 0, 4096), frame(true, past_32_bits, 3)];
        let mut file = Cursor::new(flac(0, &frames));
        assert_eq!(state_samples(&mut file).unwrap(), Some(past_32_bits + 3));
        assert_eq!(samples(&file.into_inner()[8..42]), past_32_bits + 3);
    }

    #[test]
    fn a_stated_count_is_left_and_a_broken_end_refused() {
        let frames = [frame(false, 0, 4096), frame(false, 1, 4096)];
        let stated = flac(8192, &frames);
        let mut file = Cursor::new(stated.clone());
        assert_eq!(state_samples(&mut file).unwrap(), None);
        assert_eq!(file.into_inner(), stated);

        let mut cut = flac(0, &frames);
        cut.pop();
        let mut file = Cursor::new(cut);
        assert_eq!(
            state_samples(&mut file).unwrap_err().to_string(),
            "its last frame could not be read"
        );
        assert!(state_samples(&mut Cursor::new(b"OggS".repeat(20))).is_err());
    }
}
