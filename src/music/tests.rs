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
    observe_for(&mut state, &dedicated(), audio(0.9), 2.75);
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
    for _ in 0..20 {
        observe_for(&mut state, &dedicated(), audio(0.9), 1.0);
        observe_for(&mut state, &MediaEvidence::default(), audio(0.1), 0.25);
    }
    assert!(!state.active);
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
