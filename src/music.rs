//! Local observation only: Windows facts -> evidence -> one hysteretic detector.
use bevy::prelude::*;

mod dsp;
#[cfg(target_os = "windows")]
mod native;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Default)]
pub(crate) struct MediaEvidence {
    pub available: bool,
    pub playing: bool,
    pub dedicated: bool,
    pub browser: bool,
    pub confidence: f32,
}

impl MediaEvidence {
    pub(crate) fn classify(
        source: &str,
        playing: bool,
        title: &str,
        artist: &str,
        album: &str,
        audio_type: bool,
    ) -> Self {
        let app = source.to_ascii_lowercase();
        // Small positive capability hints; unknown apps remain eligible via DSP.
        let dedicated = [
            "spotify",
            "music.ui",
            "zunemusic",
            "foobar2000",
            "aimp",
            "musicbee",
            "applemusic",
        ]
        .iter()
        .any(|s| app.contains(s));
        let browser = ["chrome", "msedge", "firefox", "opera", "brave"]
            .iter()
            .any(|s| app.contains(s));
        let metadata = if !artist.trim().is_empty() && !title.trim().is_empty() {
            0.25
        } else {
            0.0
        } + if !album.trim().is_empty() { 0.1 } else { 0.0 };
        let hint = [
            "music",
            "official audio",
            "lyrics",
            "soundtrack",
            "lofi",
            "bgm",
        ]
        .iter()
        .any(|word| title.to_ascii_lowercase().contains(word));
        let confidence: f32 = if !playing {
            0.0
        } else if dedicated {
            0.72 + metadata * 0.7
        } else if browser {
            0.15 + metadata * 0.6 + if hint { 0.05 } else { 0.0 }
        } else {
            0.15 + metadata + if audio_type { 0.2 } else { 0.0 }
        };
        Self {
            available: true,
            playing,
            dedicated,
            browser,
            confidence: confidence.min(1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AudioEvidence {
    pub available: bool,
    pub active: bool,
    pub probability: f32,
}

pub(crate) fn fused_confidence(media: &MediaEvidence, audio: AudioEvidence) -> f32 {
    if audio.available && !audio.active {
        return 0.0;
    }
    if !audio.available {
        return if media.playing && media.dedicated {
            media.confidence
        } else {
            0.0
        };
    }
    if media.playing && media.dedicated {
        return (0.8 * media.confidence + 0.2 * audio.probability).max(0.72);
    }
    // Browser title/artist alone never establishes music. Rhythmic/tonal audio
    // must carry the decision, including when metadata is absent entirely.
    if media.playing && media.browser {
        return audio.probability * 0.9 + media.confidence * 0.1;
    }
    if media.playing {
        return audio.probability * 0.85 + media.confidence * 0.15;
    }
    audio.probability * 0.9
}

#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct MusicDetectionState {
    pub(crate) active: bool,
    pub(crate) confidence: f32,
    pub(crate) media_available: bool,
    pub(crate) audio_available: bool,
    start_secs: f32,
    stop_secs: f32,
}

impl MusicDetectionState {
    pub(crate) fn observe(&mut self, media: &MediaEvidence, audio: AudioEvidence, dt: f32) {
        self.confidence = fused_confidence(media, audio);
        self.media_available = media.available;
        self.audio_available = audio.available;
        // Sustained evidence, with a separate lower hold threshold.
        if !self.active {
            self.start_secs = if self.confidence >= 0.70 {
                self.start_secs + dt
            } else {
                0.0
            };
            if self.start_secs >= 3.0 {
                self.active = true;
                self.stop_secs = 0.0;
            }
        } else {
            self.stop_secs = if self.confidence < 0.45 {
                self.stop_secs + dt
            } else {
                0.0
            };
            if self.stop_secs >= 4.0 {
                self.active = false;
                self.start_secs = 0.0;
            }
        }
    }
}

pub(crate) struct MusicPlugin;
impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MusicDetectionState>();
        #[cfg(target_os = "windows")]
        app.init_resource::<native::MusicWorker>()
            .add_systems(PreUpdate, native::poll);
    }
}
