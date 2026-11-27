//! The gate on generated audio: copies of one master, another transfer, a
//! remaster, and each kind of damage.

use super::*;
use crate::coherence::fixtures::{music, noisy, sped};
use crate::signal::filter::sosfilt;

fn scaled(x: &[[f64; 2]], gain: f64, dc: f64) -> Frames {
    x.iter()
        .map(|f| [gain * f[0] + dc, gain * f[1] + dc])
        .collect()
}

/// `x` with `count` stretches of `ms` silenced, spread over it.
fn dropped(x: &[[f64; 2]], count: usize, ms: usize) -> Frames {
    let mut y = x.to_vec();
    let len = ms * 48;
    for g in 1..=count {
        let at = g * x.len() / (count + 1);
        y[at..at + len].fill([0.0; 2]);
    }
    y
}

/// `x` with `count` single-sample impulses added, spread over it, each
/// under the copy's own peak: one at its clip level would read as clipping.
fn clicked(x: &[[f64; 2]], count: usize) -> Frames {
    let mut y = x.to_vec();
    for c in 1..=count {
        let at = c * x.len() / (count + 1);
        let sign = if y[at][0] > 0.0 { -1.0 } else { 1.0 };
        y[at] = [y[at][0] + sign * 0.3, y[at][1] + sign * 0.3];
    }
    y
}

#[test]
fn level_polarity_and_dc_part_no_copies() {
    let x = music(5.0, 21);
    let g = analyze(vec![x.clone(), noisy(&x, 0.002, 5), scaled(&x, -0.5, 0.05)]);
    assert_eq!(
        g,
        Gate {
            master: vec![0, 0, 0],
            broken: vec![None, None, None]
        }
    );
    assert_eq!(g.masters(), 1);
}

#[test]
fn another_clock_is_another_master() {
    let x = music(5.0, 22);
    let g = analyze(vec![x.clone(), noisy(&x, 0.002, 5), sped(&x, 150.0)]);
    assert_eq!(
        g.master,
        [0, 0, 1],
        "a transfer running 150 ppm fast shares no clock"
    );
    assert_eq!(g.masters(), 2);
    assert_eq!(g.broken, [None, None, None]);
}

#[test]
fn a_copy_limited_harder_is_another_master() {
    let x = music(5.0, 23);
    let limited: Frames = x
        .iter()
        .map(|f| [(4.0 * f[0]).tanh(), (4.0 * f[1]).tanh()])
        .collect();
    let g = analyze(vec![x.clone(), noisy(&x, 0.002, 5), limited]);
    assert_eq!(g.master, [0, 0, 1]);
}

#[test]
fn gaps_another_copy_plays_through_are_dropouts() {
    let x = music(5.0, 24);
    let g = analyze(vec![noisy(&x, 0.002, 5), dropped(&x, 5, 100)]);
    assert_eq!(g.master, [0, 0]);
    assert!(g.broken[0].is_none(), "{:?}", g.broken);
    assert!(
        matches!(
            g.broken[1],
            Some(Broken::Dropouts {
                gaps: 5,
                ms: 400..500
            })
        ),
        "{:?}",
        g.broken
    );
    assert!(g.broken[1].unwrap().to_string().ends_with(" s of dropouts"));
}

#[test]
fn impulses_no_other_copy_holds_are_clicks() {
    let x = music(5.0, 25);
    let g = analyze(vec![noisy(&x, 0.002, 5), clicked(&x, 12)]);
    assert_eq!(g.master, [0, 0]);
    assert!(g.broken[0].is_none(), "{:?}", g.broken);
    assert!(
        matches!(g.broken[1], Some(Broken::Clicks { count: 5.. })),
        "{:?}",
        g.broken
    );
}

#[test]
fn music_cut_off_at_full_level_is_truncated() {
    let x = music(24.0, 26);
    let g = analyze(vec![x.clone(), x[..12 * 48_000].to_vec()]);
    assert_eq!(g.master, [0, 0]);
    assert_eq!(g.broken, [None, Some(Broken::Truncated { seconds: 12 })]);
    assert_eq!(
        g.broken[1].unwrap().to_string(),
        "lacks 12 s other copies hold"
    );
}

#[test]
fn the_order_copies_come_in_changes_nothing() {
    let x = music(5.0, 27);
    let (a, b, c) = (noisy(&x, 0.002, 5), dropped(&x, 5, 100), sped(&x, 150.0));
    let one = analyze(vec![a.clone(), b.clone(), c.clone()]);
    let other = analyze(vec![c, a, b]);
    assert_eq!(one.master, [0, 0, 1]);
    assert_eq!(other.master, [0, 1, 1]);
    assert_eq!(one.broken[1], other.broken[2]);
    assert!(one.broken[1].is_some() && one.broken[0].is_none() && other.broken[1].is_none());
}

#[test]
fn fewer_than_two_copies_are_not_compared() {
    assert_eq!(analyze(Vec::new()), Gate::default());
    assert_eq!(
        analyze(vec![music(1.0, 28)]),
        Gate {
            master: vec![0],
            broken: vec![None]
        }
    );
}

#[test]
fn the_decode_asks_for_two_channels_at_48_khz() {
    let args: Vec<String> = (pcm_command(std::path::Path::new("song.flac"), true).iter())
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(
        args.windows(2)
            .any(|w| w == ["-af", "aresample=48000:resampler=soxr:precision=28"]),
        "{args:?}"
    );
    assert!(
        args.windows(2).any(|w| w == ["-ac", "2"])
            && args.ends_with(&["f32le".to_string(), "-".to_string()])
    );
    let plain = pcm_command(std::path::Path::new("song.flac"), false);
    assert!(
        plain.iter().any(|a| a == "aresample=48000"),
        "ffmpeg's own resampler"
    );
    assert_eq!(frames(&[0, 0, 128, 63, 0, 0, 0, 191]), [[1.0, -0.5]]);
}

/// What the gate finds of a copy with each fault beside a whole one and
/// another transfer, with [`VERSION`].
const WRITTEN: (&str, &str) = (
    "gate/1",
    "e6ab935cd49805965faf4b27292e8ac2c1415df094a114979189b09294071271",
);

#[test]
fn the_version_rises_with_what_it_finds() {
    let x = music(5.0, 29);
    let faults = vec![
        noisy(&x, 0.002, 5),
        dropped(&x, 5, 100),
        clicked(&x, 12),
        sped(&x, 150.0),
    ];
    let long = music(24.0, 30);
    let cut = vec![long.clone(), long[..12 * 48_000].to_vec()];
    let mut digest = crate::testing::Digest::default();
    digest.debug(&analyze(faults)).debug(&analyze(cut));
    crate::testing::check(WRITTEN, &format!("gate/{VERSION}"), &digest.hex());
}

/// A copy's two channels, unlike each other, as a whole song is decoded.
fn two_channels(n: usize) -> Frames {
    (0..n)
        .map(|i| {
            let t = real(i);
            [(t * 0.031).sin() * 0.7, (t * 0.017).cos() * 0.4 - 0.1]
        })
        .collect()
}

#[test]
fn a_copy_high_passed_as_it_streams_is_each_channel_filtered_whole() {
    let x = two_channels(20_000);
    let sos = butter(4, Band::High(6000.0), FS);
    let channel = |c: usize| sosfilt(&sos, &x.iter().map(|f| f[c]).collect::<Vec<_>>());
    let whole: Vec<[u32; 2]> = channel(0)
        .into_iter()
        .zip(channel(1))
        .map(|(l, r)| [(l as f32).to_bits(), (r as f32).to_bits()])
        .collect();
    let streamed: Vec<[u32; 2]> = hp(&x)
        .into_iter()
        .map(|[l, r]| [l.to_bits(), r.to_bits()])
        .collect();
    assert_eq!(streamed, whole);
}

#[test]
fn a_copy_s_level_read_as_it_streams_is_its_mono_s() {
    let x = two_channels(20_000);
    let m = mono(&x);
    let whole: Vec<u64> = (0..m.len() / 480)
        .map(|w| {
            let power = m[w * 480..(w + 1) * 480].iter().map(|v| v * v).sum::<f64>();
            (10.0 * (power / 480.0 + 1e-14).log10()).to_bits()
        })
        .collect();
    let streamed: Vec<u64> = env(&x).into_iter().map(f64::to_bits).collect();
    assert_eq!(streamed, whole);
}
