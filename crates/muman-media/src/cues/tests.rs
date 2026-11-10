use super::*;

const SONG: &str = "[00:01.00]Paper comets\n[00:03.50]Lantern weather\n";

#[test]
fn a_line_is_a_cue_until_the_next_and_the_last_until_the_song_ends() {
    assert_eq!(
        write(SONG, LyricsFormat::Vtt, true, Some(10_000)).unwrap(),
        "WEBVTT\n\n00:00:01.000 --> 00:00:03.500\nPaper comets\n\n\
         00:00:03.500 --> 00:00:10.000\nLantern weather\n"
    );
    assert_eq!(
        write(SONG, LyricsFormat::Srt, true, Some(10_000)).unwrap(),
        "1\n00:00:01,000 --> 00:00:03,500\nPaper comets\n\n\
         2\n00:00:03,500 --> 00:00:10,000\nLantern weather\n"
    );
    let unknown = write(SONG, LyricsFormat::Srt, true, None).unwrap();
    assert!(
        unknown.ends_with("00:00:03,500 --> 00:00:07,500\nLantern weather\n"),
        "{unknown}"
    );
    let early = write(SONG, LyricsFormat::Srt, true, Some(2000)).unwrap();
    assert_eq!(early, unknown, "a length before the last line is none");
}

#[test]
fn a_line_sung_twice_is_two_cues_and_lines_at_one_time_are_one() {
    let lrc = "[00:05.00][00:01.00]Chorus\n[00:01.00](backing)\n[00:02.00]\n[00:03.00]Verse\n";
    assert_eq!(
        write(lrc, LyricsFormat::Srt, true, Some(9000)).unwrap(),
        "1\n00:00:01,000 --> 00:00:02,000\nChorus\n(backing)\n\n\
         2\n00:00:03,000 --> 00:00:05,000\nVerse\n\n\
         3\n00:00:05,000 --> 00:00:09,000\nChorus\n",
        "an empty line ends the cue before it"
    );
    assert_eq!(
        write(lrc, LyricsFormat::Txt, true, None).unwrap(),
        "Chorus\n(backing)\nVerse\nChorus\n"
    );
}

#[test]
fn a_word_s_time_is_kept_inside_its_cue_and_in_order() {
    let lrc = "[00:01.00]<00:01.00>Paper <00:01.50>comets <00:01.20>far <00:04.00>off\n\
               [00:03.00]Lantern\n";
    assert_eq!(
        write(lrc, LyricsFormat::Vtt, true, Some(6000)).unwrap(),
        "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\n\
         Paper <00:00:01.500>comets far off\n\n\
         00:00:03.000 --> 00:00:06.000\nLantern\n"
    );
    let plain = write(lrc, LyricsFormat::Vtt, false, Some(6000)).unwrap();
    assert!(plain.contains("\nPaper comets far off\n"), "{plain}");
    assert!(
        write(lrc, LyricsFormat::Srt, true, Some(6000))
            .unwrap()
            .contains("\nPaper comets far off\n")
    );
    assert_eq!(
        write(lrc, LyricsFormat::Lrc, false, None).unwrap(),
        "[00:01.00]Paper comets far off\n[00:03.00]Lantern\n"
    );
    assert_eq!(
        write(lrc, LyricsFormat::Lrc, true, None).unwrap(),
        lrc,
        "LRC as kept"
    );
}

#[test]
fn what_webvtt_reads_as_markup_is_escaped() {
    let lrc = "[00:01.00]a < b & c --> d\n";
    assert_eq!(
        write(lrc, LyricsFormat::Vtt, true, Some(3000)).unwrap(),
        "WEBVTT\n\n00:00:01.000 --> 00:00:03.000\na &lt; b &amp; c --&gt; d\n"
    );
}

#[test]
fn untimed_lyrics_make_no_cues_and_plain_text_drops_id_tags() {
    let untimed = "[ar:Marlo Venn]\nPaper comets\n\n  Lantern weather\n";
    assert_eq!(write(untimed, LyricsFormat::Vtt, true, Some(9000)), None);
    assert_eq!(write(untimed, LyricsFormat::Srt, true, Some(9000)), None);
    assert_eq!(
        write(untimed, LyricsFormat::Txt, true, None).unwrap(),
        "Paper comets\nLantern weather\n"
    );
    assert_eq!(
        write(untimed, LyricsFormat::Lrc, true, None).unwrap(),
        untimed
    );
}

/// What reading, moving and writing make of lyrics in four encodings and
/// with control characters,
/// with [`crate::lyrics::WRITING`].
const WRITTEN: (&str, &str) = (
    "5",
    "a3be44f40152fb608400e294577854ed540ec7b13c37f1d509374aa554f00a7a",
);

#[test]
fn writing_rises_with_what_it_makes() {
    let lrc = "[ar:Lantern Hollow]\r\n[00:01.20]<00:01.20>Paper <00:01.80>comets\r\n\
               [00:04.50](Guitar solo)\r\n[00:09.05]on Kessler Row\r\n";
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain(lrc.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let sources = [
        lrc.as_bytes().to_vec(),
        utf16,
        encoding_rs::WINDOWS_1252
            .encode("[00:02.00]Caf\u{e9} lights\n")
            .0
            .into_owned(),
        encoding_rs::SHIFT_JIS
            .encode("[00:03.00]\u{591c}\u{306e}\u{706f}\u{53f0}\n")
            .0
            .into_owned(),
        b"[00:03,50]Comma lights\n[00:99.99]Past the minute\n".to_vec(),
        b"[offset:+500]\n[offset:-900]\n[00:04.00]Two offsets\n".to_vec(),
        b"[00:04.00]Near\n[00:10.00][59:00.00]Far past\n[00:30.00]Gone\n".to_vec(),
        b"[00:05.00]Bell\x07 and \x1b[1mbold\x1b[0m\r\n[00:06.00]End\r\n\x1a".to_vec(),
    ];
    let mut digest = crate::testing::Digest::default();
    digest.debug(&crate::facts::LRC_ARGS);
    for bytes in sources {
        let text = crate::lyrics::decode(&bytes);
        digest.text(&text);
        for (offset_ms, stretch_ppm) in [(0, 0), (1500, 0), (-700, 2500)] {
            let moved = crate::lyrics::shift_lrc(&text, offset_ms, stretch_ppm);
            let (moved, _) = crate::lyrics::until(&moved, 9_000);
            digest.text(&moved);
            for format in [
                LyricsFormat::Lrc,
                LyricsFormat::Vtt,
                LyricsFormat::Srt,
                LyricsFormat::Txt,
            ] {
                for words in [true, false] {
                    for end_ms in [None, Some(12_000)] {
                        digest.debug(&write(&moved, format, words, end_ms));
                    }
                }
            }
        }
    }
    crate::testing::check(WRITTEN, &crate::lyrics::WRITING.to_string(), &digest.hex());
}
