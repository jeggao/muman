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
