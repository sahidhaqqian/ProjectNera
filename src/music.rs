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
    pub source: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playback_type_music: bool,
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
            "official video",
            "lyric video",
            "lyrics",
            "soundtrack",
            "lofi",
            "bgm",
        ]
        .iter()
        .any(|word| title.to_ascii_lowercase().contains(word));
        let title_lower = title.to_ascii_lowercase();
        let spoken = [
            "tutorial",
            "podcast",
            "interview",
            "lecture",
            "how to",
            "explained",
            "news",
        ]
        .iter()
        .any(|word| title_lower.contains(word));
        let pair = !title.trim().is_empty() && !artist.trim().is_empty();
        let structured = metadata >= 0.35 && !spoken;
        let meaningful = hint && !spoken;
        let confidence: f32 = if !playing {
            0.0
        // Chromium advertises Music for ordinary videos/Shorts too. Preserve
        // the fact, but require independent music metadata outside music apps.
        // A YouTube title + channel name is not structured song metadata.
        } else if audio_type && (dedicated || structured || meaningful && pair) {
            0.95
        } else if dedicated {
            0.72 + metadata * 0.7
        } else if structured {
            0.90
        } else if meaningful {
            0.82
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
            source: source.into(),
            title: title.trim().into(),
            artist: artist.trim().into(),
            album: album.trim().into(),
            playback_type_music: audio_type,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ConfidenceTier {
    High,
    Medium,
    #[default]
    Ambiguous,
}
impl ConfidenceTier {
    fn start_seconds(self) -> f32 {
        match self {
            Self::High => 0.5,
            Self::Medium => 1.0,
            Self::Ambiguous => 2.5,
        }
    }
}

fn fuse(media: &MediaEvidence, audio: AudioEvidence) -> (f32, ConfidenceTier) {
    use ConfidenceTier::*;
    if audio.available && !audio.active {
        return (0.0, Ambiguous);
    }
    if !audio.available {
        return (
            if media.playing && media.dedicated {
                media.confidence
            } else {
                0.0
            },
            Ambiguous,
        );
    }
    // Strong Media Session facts survive browser classification. Output must
    // actually be active; title/channel alone never qualifies for the fast path.
    if media.playing && media.confidence >= 0.90 {
        return (
            (0.8 * media.confidence + 0.2 * audio.probability).max(0.72),
            High,
        );
    }
    if media.playing && media.dedicated {
        return (
            (0.8 * media.confidence + 0.2 * audio.probability).max(0.72),
            High,
        );
    }
    if media.playing && media.confidence >= 0.82 && audio.probability >= 0.45 {
        return (0.7 * media.confidence + 0.3 * audio.probability, Medium);
    }
    // Weak metadata retains the original DSP-dominated admission threshold.
    if media.playing && media.browser {
        return (
            audio.probability * 0.9 + media.confidence.min(0.40) * 0.1,
            Ambiguous,
        );
    }
    if media.playing {
        return (
            audio.probability * 0.85 + media.confidence.min(0.55) * 0.15,
            Ambiguous,
        );
    }
    (audio.probability * 0.9, Ambiguous)
}
#[cfg(test)]
pub(crate) fn fused_confidence(media: &MediaEvidence, audio: AudioEvidence) -> f32 {
    fuse(media, audio).0
}

#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct MusicDetectionState {
    pub(crate) active: bool,
    pub(crate) confidence: f32,
    pub(crate) media_available: bool,
    pub(crate) audio_available: bool,
    pub(crate) tier: ConfidenceTier,
    start_secs: f32,
    stop_secs: f32,
    media_key: Option<(String, String, String, String)>,
    confirmed_media_key: Option<(String, String, String, String)>,
    needs_confirmation: bool,
}

impl MusicDetectionState {
    pub(crate) fn observe(&mut self, media: &MediaEvidence, audio: AudioEvidence, dt: f32) {
        let (confidence, tier) = fuse(media, audio);
        let key = (
            media.source.clone(),
            media.title.clone(),
            media.artist.clone(),
            media.album.clone(),
        );
        if self.media_key.as_ref() != Some(&key) {
            self.needs_confirmation =
                self.active && self.confirmed_media_key.as_ref() != Some(&key);
            self.media_key = Some(key);
            self.start_secs = 0.0;
        }
        if (!self.active || self.needs_confirmation) && self.tier != tier {
            // A weaker tier must earn its own sustained evidence, rather than
            // borrowing time accumulated under a faster rule (or vice versa).
            self.start_secs = 0.0;
        }
        self.confidence = confidence;
        self.tier = tier;
        self.media_available = media.available;
        self.audio_available = audio.available;
        // Sustained evidence, with a separate lower hold threshold.
        if !self.active || self.needs_confirmation {
            self.start_secs = if self.confidence >= 0.70 {
                self.start_secs + dt
            } else {
                0.0
            };
            if self.start_secs >= self.tier.start_seconds() {
                self.active = true;
                self.confirmed_media_key = self.media_key.clone();
                self.needs_confirmation = false;
                self.stop_secs = 0.0;
            }
        }
        if self.active {
            // The lower hold threshold belongs to the confirmed media item.
            // A new video must establish music; scrolling Shorts cannot keep
            // refreshing the previous song's grace period indefinitely.
            self.stop_secs = if self.needs_confirmation || self.confidence < 0.45 {
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
