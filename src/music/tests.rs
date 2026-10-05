use super::*;
fn audio(probability: f32) -> AudioEvidence {
    AudioEvidence {
        available: true,
        active: true,
        probability,
    }
}
fn dedicated() -> MediaEvidence {
    MediaEvidence::classify("Spotify.exe", true, "Track", "Artist", "Album", true)
}
fn observe_for(
    state: &mut MusicDetectionState,
    media: &MediaEvidence,
    audio: AudioEvidence,
    seconds: f32,
) {
    for _ in 0..(seconds * 4.0) as usize {
        state.observe(media, audio, 0.25);
    }
}
#[test]
fn media_dedicated_browser_incomplete_paused_and_empty() {
    assert!(dedicated().confidence > 0.9);
    let incomplete = MediaEvidence::classify("Spotify.exe", true, "", "", "", false);
    assert!(incomplete.confidence >= 0.7);
    let browser = MediaEvidence::classify(
        "chrome.exe",
        true,
        "Official Music Audio tutorial",
        "",
        "",
        false,
    );
    assert!(browser.browser);
    assert!(browser.confidence < 0.4);
    assert_eq!(
        MediaEvidence::classify("Spotify.exe", false, "Track", "Artist", "", true).confidence,
        0.0
    );
    assert!(!MediaEvidence::default().playing);
}
#[test]
fn fusion_requires_context_or_musiclike_audio_and_degrades_independently() {
    assert!(fused_confidence(&dedicated(), audio(0.2)) >= 0.7);
    let browser = MediaEvidence::classify("msedge.exe", true, "Song", "", "", false);
    assert!(fused_confidence(&browser, audio(0.95)) >= 0.7);
    assert!(fused_confidence(&browser, audio(0.15)) < 0.45);
    assert!(fused_confidence(&MediaEvidence::default(), audio(0.2)) < 0.45);
    assert!(
        fused_confidence(
            &MediaEvidence::classify("Spotify.exe", true, "", "", "", false),
            audio(0.9)
        ) >= 0.7
    );
    assert!(fused_confidence(&MediaEvidence::default(), audio(0.95)) >= 0.7);
    assert!(fused_confidence(&dedicated(), AudioEvidence::default()) >= 0.7);
    assert_eq!(
        fused_confidence(&MediaEvidence::default(), AudioEvidence::default()),
        0.0
    );
    assert_eq!(
        fused_confidence(
            &dedicated(),
            AudioEvidence {
                available: true,
                ..Default::default()
            }
        ),
        0.0
    );
}
#[test]
fn hysteresis_sustained_start_brief_silence_sustained_stop_and_noisy_flapping() {
    let mut state = MusicDetectionState::default();
    observe_for(&mut state, &dedicated(), audio(0.9), 0.25);
    assert!(!state.active);
    observe_for(&mut state, &dedicated(), audio(0.9), 0.25);
    assert!(state.active);
    observe_for(
        &mut state,
        &MediaEvidence::default(),
        AudioEvidence::default(),
        2.0,
    );
    assert!(state.active);
    observe_for(&mut state, &dedicated(), audio(0.9), 0.25);
    observe_for(
        &mut state,
        &MediaEvidence::default(),
        AudioEvidence::default(),
        3.75,
    );
    assert!(state.active);
    observe_for(
        &mut state,
        &MediaEvidence::default(),
        AudioEvidence::default(),
        0.25,
    );
    assert!(!state.active);
    let ambiguous = MediaEvidence::classify("chrome.exe", true, "Track", "", "", false);
    for _ in 0..20 {
        observe_for(&mut state, &ambiguous, audio(0.9), 1.0);
        observe_for(&mut state, &MediaEvidence::default(), audio(0.1), 0.25);
    }
    assert!(!state.active);
}
#[test]
fn browser_music_type_preserved_and_fast_even_with_weak_dsp() {
    let media = MediaEvidence::classify("chrome.exe", true, "Song", "Artist", "Album", true);
    assert_eq!(
        (&media.title[..], &media.artist[..], &media.album[..]),
        ("Song", "Artist", "Album")
    );
    assert!(media.playback_type_music);
    assert_eq!(fuse(&media, audio(0.1)).1, ConfidenceTier::High);
    let mut state = MusicDetectionState::default();
    observe_for(&mut state, &media, audio(0.1), 0.25);
    assert!(!state.active);
    observe_for(&mut state, &media, audio(0.1), 0.25);
    assert!(state.active);
    let silent = AudioEvidence {
        available: true,
        ..Default::default()
    };
    assert_eq!(fused_confidence(&media, silent), 0.0);
    assert_eq!(fused_confidence(&media, AudioEvidence::default()), 0.0);
    let paused = MediaEvidence::classify("chrome.exe", false, "Song", "Artist", "Album", true);
    assert_ne!(fuse(&paused, audio(0.5)).1, ConfidenceTier::High);
}
#[test]
fn browser_structured_metadata_and_moderate_dsp_use_high_or_medium() {
    let album = MediaEvidence::classify("msedge.exe", true, "Song", "Artist", "Album", false);
    assert_eq!(fuse(&album, audio(0.45)).1, ConfidenceTier::High);
    let media = MediaEvidence::classify(
        "msedge.exe",
        true,
        "Song (Official Audio)",
        "Artist",
        "",
        false,
    );
    assert_eq!(fuse(&media, audio(0.45)).1, ConfidenceTier::Medium);
    assert!(fused_confidence(&media, audio(0.45)) >= 0.70);
    let mut state = MusicDetectionState::default();
    observe_for(&mut state, &media, audio(0.45), 0.75);
    assert!(!state.active);
    observe_for(&mut state, &media, audio(0.45), 0.25);
    assert!(state.active);
    assert!(fused_confidence(&media, audio(0.2)) < 0.45);
}
#[test]
fn browser_tutorial_podcast_and_weak_audio_do_not_start() {
    for title in [
        "How to make music tutorial",
        "Podcast interview",
        "A conversation",
    ] {
        let media = MediaEvidence::classify("chrome.exe", true, title, "Channel", "", false);
        let mut state = MusicDetectionState::default();
        observe_for(&mut state, &media, audio(0.25), 10.0);
        assert!(!state.active, "{title}");
    }
    let tutorial = MediaEvidence::classify(
        "chrome.exe",
        true,
        "Music tutorial",
        "Channel",
        "Series",
        false,
    );
    assert_eq!(fuse(&tutorial, audio(0.5)).1, ConfidenceTier::Ambiguous);
    assert!(fused_confidence(&tutorial, audio(0.5)) < 0.70);
}
#[test]
fn generic_browser_music_flag_and_channel_are_not_song_metadata() {
    // Real Chromium sessions mark ordinary YouTube videos as Music, too.
    for source in ["Chrome", "msedge.exe", "EmbeddedWebPlayer"] {
        for title in [
            "Everyone Thinks I Had a BBL 😭",
            "A conversation #shorts",
            "Music tutorial",
        ] {
            let media = MediaEvidence::classify(source, true, title, "Creator", "", true);
            assert!(media.playback_type_music);
            for probability in [0.25, 0.40, 0.45, 0.60] {
                let mut state = MusicDetectionState::default();
                observe_for(&mut state, &media, audio(probability), 10.0);
                assert!(!state.active, "{source}: {title}, DSP={probability}");
                assert_eq!(state.tier, ConfidenceTier::Ambiguous);
            }
        }
    }
}
#[test]
fn official_music_video_keeps_fast_start_without_album() {
    for source in ["Chrome", "EmbeddedWebPlayer"] {
        let media = MediaEvidence::classify(
            source,
            true,
            "Coldplay - Adventure Of A Lifetime (Official Video)",
            "Coldplay",
            "",
            true,
        );
        let mut state = MusicDetectionState::default();
        observe_for(&mut state, &media, audio(0.1), 0.25);
        assert!(!state.active);
        observe_for(&mut state, &media, audio(0.1), 0.25);
        assert!(state.active);
        assert_eq!(state.tier, ConfidenceTier::High);
    }
}
#[test]
fn song_to_scrolling_shorts_cannot_inherit_music_hold_threshold() {
    let song = MediaEvidence::classify("Chrome", true, "Song (Official Audio)", "Artist", "", true);
    let short = MediaEvidence::classify("Chrome", true, "A conversation", "Creator", "", true);
    let next = MediaEvidence::classify("Chrome", true, "Another video", "Creator", "", true);
    let mut state = MusicDetectionState::default();
    observe_for(&mut state, &song, audio(0.6), 0.5);
    assert!(state.active);
    // DSP is above the old hold threshold, but insufficient to establish music.
    observe_for(&mut state, &short, audio(0.6), 2.0);
    assert!(state.active);
    observe_for(&mut state, &next, audio(0.6), 1.75);
    assert!(state.active);
    observe_for(&mut state, &next, audio(0.6), 0.25);
    assert!(!state.active);
    observe_for(&mut state, &next, audio(0.6), 8.0);
    assert!(!state.active);
    observe_for(&mut state, &song, audio(0.6), 0.5);
    assert!(state.active);
}
#[test]
fn confirmed_music_can_hold_but_new_ambiguous_items_must_qualify() {
    let song = MediaEvidence::classify("Chrome", true, "Song", "Artist", "", true);
    let next = MediaEvidence::classify("Chrome", true, "Next song", "Artist", "", true);
    let mut state = MusicDetectionState::default();
    observe_for(&mut state, &song, audio(0.9), 2.5);
    assert!(state.active);
    observe_for(&mut state, &song, audio(0.6), 6.0);
    assert!(state.active);
    observe_for(&mut state, &next, audio(0.9), 2.5);
    assert!(state.active);
    observe_for(&mut state, &next, audio(0.6), 6.0);
    assert!(state.active);
    // Separate short items cannot pool their start duration either.
    state = MusicDetectionState::default();
    for _ in 0..4 {
        observe_for(&mut state, &song, audio(0.9), 1.5);
        observe_for(&mut state, &next, audio(0.9), 1.5);
        assert!(!state.active);
    }
}
#[test]
fn ambiguous_browser_uses_slow_path_and_tier_changes_restart_accumulation() {
    let weak = MediaEvidence::classify("chrome.exe", true, "Video", "", "", false);
    let mut state = MusicDetectionState::default();
    observe_for(&mut state, &weak, audio(0.9), 2.25);
    assert_eq!(state.tier, ConfidenceTier::Ambiguous);
    assert!(!state.active);
    observe_for(&mut state, &weak, audio(0.9), 0.25);
    assert!(state.active);
    state = MusicDetectionState::default();
    observe_for(&mut state, &dedicated(), audio(0.9), 0.25);
    observe_for(&mut state, &weak, audio(0.9), 2.25);
    assert!(!state.active);
    observe_for(&mut state, &weak, audio(0.9), 0.25);
    assert!(state.active);
}
#[test]
fn all_tiers_hold_brief_silence_and_stop_after_four_seconds() {
    for media in [
        dedicated(),
        MediaEvidence::classify("chrome.exe", true, "Song", "Artist", "", false),
        MediaEvidence::classify("chrome.exe", true, "Video", "", "", false),
    ] {
        let mut state = MusicDetectionState::default();
        observe_for(&mut state, &media, audio(0.9), 3.0);
        assert!(state.active);
        observe_for(&mut state, &MediaEvidence::default(), audio(0.1), 3.75);
        assert!(state.active);
        observe_for(&mut state, &media, audio(0.9), 0.25);
        observe_for(&mut state, &MediaEvidence::default(), audio(0.1), 3.75);
        assert!(state.active);
        observe_for(&mut state, &MediaEvidence::default(), audio(0.1), 0.25);
        assert!(!state.active);
    }
}
fn analyze(mut signal: impl FnMut(f32) -> f32) -> AudioEvidence {
    let mut analyzer = dsp::Analyzer::default();
    for index in 0..40000 {
        let t = index as f32 / 8000.0;
        analyzer.push(signal(t), 8000);
    }
    analyzer.evidence()
}
#[test]
fn dsp_silence_constant_tone_and_periodic_harmonic_music() {
    assert!(!analyze(|_| 0.0).active);
    let tone = analyze(|t| 0.15 * (t * 440.0 * std::f32::consts::TAU).sin());
    assert!(tone.active);
    assert!(tone.probability < 0.7, "{tone:?}");
    let music = analyze(|t| {
        let beat = 0.15 + 0.85 * (-((t * 2.0).fract() * 8.0)).exp();
        beat * 0.2
            * ((t * 220.0 * std::f32::consts::TAU).sin()
                + 0.5 * (t * 440.0 * std::f32::consts::TAU).sin()
                + 0.3 * (t * 880.0 * std::f32::consts::TAU).sin())
    });
    assert!(music.active);
    assert!(music.probability > 0.78, "{music:?}");
}
#[test]
fn irregular_bursts_and_noise_do_not_establish_music() {
    let mut rng = crate::rng::NeraRng::seeded(42);
    let noise = analyze(|_| (rng.next_f32() - 0.5) * 0.1);
    assert!(noise.probability < 0.7, "{noise:?}");
    let effects = analyze(|t| {
        if [0.1, 0.9, 2.3, 3.1, 4.7]
            .iter()
            .any(|start| t >= *start && t < start + 0.06)
        {
            (rng.next_f32() - 0.5) * 0.6
        } else {
            0.0
        }
    });
    assert!(effects.probability < 0.45, "{effects:?}");
}
