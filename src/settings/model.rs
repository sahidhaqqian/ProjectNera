//! Persistent settings schema, defaults, and backward-compatible alarm encoding.
use bevy::{
    audio::Volume,
    prelude::*,
    settings::{ReflectSettingsGroup, SettingsGroup},
};

// =============================================================================
// Persistent state
// =============================================================================

/// Persistent alarm entry used by the multi-alarm scheduler.
///
/// The Settings framework only persists `alarm_book` as a compact string. This
/// keeps the general settings schema simple while still allowing a variable
/// number of alarms without adding another serialization dependency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AlarmConfig {
    pub(crate) id: u32,
    pub(crate) enabled: bool,
    pub(crate) label: String,
    pub(crate) hour: u8,
    pub(crate) minute: u8,
    /// 0 = one-time date, 1 = weekly repeat.
    pub(crate) mode: u8,
    pub(crate) year: u16,
    pub(crate) month: u8,
    pub(crate) day: u8,
    /// Monday = bit 0 ... Sunday = bit 6.
    pub(crate) repeat_days: u8,
    /// Stable built-in/imported ID; u16::MAX selects Random at trigger time.
    pub(crate) sound: u16,
}

impl AlarmConfig {
    pub(crate) fn default_new(id: u32) -> Self {
        let (year, month, day) = default_alarm_date();

        Self {
            id,
            enabled: true,
            label: "New Alarm".to_string(),
            hour: 7,
            minute: 30,
            mode: 1,
            year,
            month,
            day,
            repeat_days: 0b0111_1111,
            sound: 1,
        }
    }

    pub(crate) fn normalized(mut self) -> Self {
        self.hour = self.hour.min(23);
        self.minute = self.minute.min(59);
        self.mode = if self.mode == 0 { 0 } else { 1 };
        self.month = self.month.clamp(1, 12);
        self.day = self
            .day
            .clamp(1, alarm_days_in_month(self.year, self.month));
        self.repeat_days &= 0b0111_1111;
        if self.mode == 1 && self.repeat_days == 0 {
            self.enabled = false;
        }

        let trimmed = self.label.trim();
        self.label = if trimmed.is_empty() {
            "Alarm".to_string()
        } else {
            trimmed.chars().take(32).collect()
        };

        self
    }

    pub(crate) fn day_enabled(&self, bit: u8) -> bool {
        self.repeat_days & (1 << bit.min(6)) != 0
    }

    pub(crate) fn toggle_day(&mut self, bit: u8) {
        self.repeat_days ^= 1 << bit.min(6);
    }

    pub(crate) fn cycle_hour(&mut self) {
        self.hour = (self.hour + 1) % 24;
    }

    pub(crate) fn decrement_hour(&mut self) {
        self.hour = if self.hour == 0 { 23 } else { self.hour - 1 };
    }

    pub(crate) fn cycle_minute(&mut self) {
        if self.minute >= 59 {
            self.minute = 0;
            self.hour = (self.hour + 1) % 24;
        } else {
            self.minute += 1;
        }
    }

    pub(crate) fn decrement_minute(&mut self) {
        if self.minute == 0 {
            self.minute = 59;
            self.decrement_hour();
        } else {
            self.minute -= 1;
        }
    }

    pub(crate) fn cycle_mode(&mut self) {
        self.mode = if self.mode == 0 { 1 } else { 0 };
        if self.mode == 1 {
            // Entering weekly mode starts from the intuitive alarm-app default:
            // every weekday button is ON until the user turns specific days OFF.
            self.repeat_days = 0b0111_1111;
        }
    }

    pub(crate) fn cycle_year(&mut self) {
        #[cfg(target_os = "windows")]
        let current_year = crate::platform::local_datetime().year;

        #[cfg(not(target_os = "windows"))]
        let current_year = 2026;

        self.year = if self.year < current_year || self.year >= current_year.saturating_add(5) {
            current_year
        } else {
            self.year.saturating_add(1)
        };

        self.day = self.day.min(alarm_days_in_month(self.year, self.month));
    }

    pub(crate) fn decrement_year(&mut self) {
        #[cfg(target_os = "windows")]
        let current_year = crate::platform::local_datetime().year;

        #[cfg(not(target_os = "windows"))]
        let current_year = 2026;

        self.year = self.year.saturating_sub(1).max(current_year);
        self.day = self.day.min(alarm_days_in_month(self.year, self.month));
    }

    pub(crate) fn cycle_month(&mut self) {
        self.month = if self.month >= 12 {
            1
        } else {
            self.month.max(1) + 1
        };
        self.day = self.day.min(alarm_days_in_month(self.year, self.month));
    }

    pub(crate) fn decrement_month(&mut self) {
        self.month = if self.month <= 1 { 12 } else { self.month - 1 };
        self.day = self.day.min(alarm_days_in_month(self.year, self.month));
    }

    pub(crate) fn cycle_day(&mut self) {
        let max_day = alarm_days_in_month(self.year, self.month);
        self.day = if self.day >= max_day {
            1
        } else {
            self.day.max(1) + 1
        };
    }

    pub(crate) fn decrement_day(&mut self) {
        let max_day = alarm_days_in_month(self.year, self.month);
        self.day = if self.day <= 1 {
            max_day
        } else {
            self.day.min(max_day) - 1
        };
    }

    pub(crate) fn cycle_label(&mut self) {
        const LABELS: [&str; 7] = [
            "Alarm", "Wake up", "Work", "Meeting", "Study", "Break", "Reminder",
        ];

        let current = LABELS
            .iter()
            .position(|candidate| *candidate == self.label)
            .unwrap_or(0);
        self.label = LABELS[(current + 1) % LABELS.len()].to_string();
    }

    pub(crate) fn mode_label(&self) -> &'static str {
        if self.mode == 0 {
            "One-time"
        } else {
            "Repeat weekly"
        }
    }

    pub(crate) fn date_label(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Behavior cadence independent of how primitive settings are persisted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActivityLevel {
    Calm,
    Normal,
    Active,
}

impl ActivityLevel {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Calm => "Calm",
            Self::Normal => "Normal",
            Self::Active => "Active",
        }
    }

    pub(crate) fn walk_interval_secs(self) -> (f32, f32) {
        match self {
            Self::Calm => (60.0, 180.0),
            Self::Normal => (30.0, 90.0),
            Self::Active => (15.0, 45.0),
        }
    }
}

/// Settings that already have a real runtime consumer.
///
/// Keep unfinished controls out of this resource: a setting should only be
/// persisted once the engine actually knows how to apply it.
#[derive(Resource, SettingsGroup, Reflect, Clone, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "general")]
pub(crate) struct NeraSettings {
    pub(crate) always_on_top: bool,
    /// When enabled, the primary Nera window stays hidden on launch until the
    /// tray's Show Nera command makes it visible.
    pub(crate) start_hidden: bool,
    pub(crate) remember_position: bool,
    pub(crate) window_x: i32,
    pub(crate) window_y: i32,

    /// Uniform VRM scale in percent. Kept proportional; no independent X/Y stretch.
    pub(crate) character_size: u16,
    /// Empty selects bundled Nera; imported models use the private characters asset source.
    pub(crate) character_asset: String,
    /// Passive saved body-mode preference. `AvatarBodyMode` owns runtime mode;
    /// its proportion layer restores this after humanoid data becomes ready.
    pub(crate) body_mode: String,
    /// Standing support offset in logical pixels. Positive values keep the visible
    /// feet slightly above the desktop support line; negative values sink them lower.
    pub(crate) ground_offset: i16,

    /// Allows Nera to walk around the desktop while otherwise idle.
    /// Disabled by default, including profiles created before walking existed.
    pub(crate) walking_enabled: bool,
    /// Cursor play observes only; it never captures or moves the mouse.
    pub(crate) cursor_play: bool,
    /// Requires Allow Roaming and an exposed, safe monitor edge.
    pub(crate) edge_peek: bool,
    /// Opt-in local Windows music observation; MusicDetectionState owns evidence.
    pub(crate) audio_reactive_dance: bool,
    /// Last Focus choices only. A running session never survives process exit.
    pub(crate) focus_mode: u8,
    pub(crate) focus_duration: u8,
    pub(crate) focus_custom_minutes: u16,
    pub(crate) gravity_enabled: bool,
    pub(crate) throw_enabled: bool,
    /// Launch strength percentage. 100 preserves the original throw velocity.
    pub(crate) throw_intensity: u8,
    /// 0 = Calm, 1 = Normal, 2 = Active. Unknown values use Normal.
    pub(crate) activity_level: u8,
    /// Maximum distance from the last user placement, in logical pixels (DPI-scaled).
    pub(crate) roaming_radius_px: u16,
    /// Pause autonomous movement while a foreground app covers its monitor.
    pub(crate) pause_roaming_fullscreen: bool,

    /// Master switch for gaze behavior driven by the global mouse cursor.
    pub(crate) follow_mouse: bool,
    /// Enables VRM LookAt eye tracking while Follow Mouse is active.
    pub(crate) follow_eyes: bool,
    /// Extends LookAt into bevy_vrm1 upper-body tracking.
    pub(crate) follow_head: bool,
    /// Cursor-to-gaze range in percent. UI cycles 25 / 50 / 75 / 100.
    pub(crate) follow_intensity: u8,

    /// Master switch for the desktop-friendly shadow system.
    pub(crate) shadow_enabled: bool,
    /// 0 = Simple blob, 1 = real animated VRM Character shadow.
    pub(crate) shadow_type: u8,
    /// Overall alpha in percent. UI cycles 25 / 40 / 55 / 70.
    pub(crate) shadow_opacity: u8,
    /// Controls how far the layered translucent rings spread.
    /// UI cycles 25 / 50 / 75 / 100.
    pub(crate) shadow_softness: u8,
    /// Key-light intensity percentage. UI cycles 75 / 100 / 125 / 150.
    pub(crate) brightness: u8,

    /// Global application audio level in percent.
    pub(crate) master_volume: u8,
    /// Keeps the configured volume while silencing all current/future Nera audio.
    pub(crate) audio_muted: bool,

    /// 0 = uncapped update loop. Rendering can still be limited by VSync
    /// because the primary Nera window currently uses AutoVsync.
    pub(crate) fps_limit: u16,

    // Phase 5.76 — persistent custom alarm (single-alarm foundation).
    pub(crate) alarm_enabled: bool,
    pub(crate) alarm_hour: u8,
    pub(crate) alarm_minute: u8,
    /// 0 = one-time date, 1 = weekly repeat.
    pub(crate) alarm_mode: u8,
    pub(crate) alarm_year: u16,
    pub(crate) alarm_month: u8,
    pub(crate) alarm_day: u8,
    /// Monday = bit 0 ... Sunday = bit 6.
    pub(crate) alarm_repeat_days: u8,

    /// Phase 5.77 — variable-size alarm book.
    ///
    /// Format is versioned and intentionally string-backed so the persistent
    /// SettingsGroup stays compatible with primitive reflected types.
    pub(crate) alarm_book: String,
    /// False means this profile predates the multi-alarm book and should expose
    /// the legacy single-alarm fields as one migration candidate.
    pub(crate) alarm_book_initialized: bool,
    pub(crate) alarm_sound: u16,
}

fn default_alarm_date() -> (u16, u8, u8) {
    #[cfg(target_os = "windows")]
    {
        let now = crate::platform::local_datetime();
        return (now.year, now.month as u8, now.day as u8);
    }

    #[cfg(not(target_os = "windows"))]
    {
        (2026, 1, 1)
    }
}

pub(crate) fn alarm_days_in_month(year: u16, month: u8) -> u8 {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);

    match month.clamp(1, 12) {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 31,
    }
}

impl Default for NeraSettings {
    fn default() -> Self {
        let (alarm_year, alarm_month, alarm_day) = default_alarm_date();

        Self {
            always_on_top: true,
            start_hidden: false,
            remember_position: true,
            window_x: 200,
            window_y: 200,
            character_size: 100,
            character_asset: String::new(),
            body_mode: "normal".to_string(),
            ground_offset: 0,
            walking_enabled: false,
            cursor_play: true,
            edge_peek: true,
            audio_reactive_dance: false,
            focus_mode: 0,
            focus_duration: 0,
            focus_custom_minutes: 35,
            gravity_enabled: true,
            throw_enabled: true,
            throw_intensity: 50,
            activity_level: 1,
            roaming_radius_px: 400,
            pause_roaming_fullscreen: true,
            follow_mouse: true,
            follow_eyes: true,
            follow_head: true,
            follow_intensity: 25,
            shadow_enabled: true,
            shadow_type: 0,
            shadow_opacity: 40,
            shadow_softness: 75,
            brightness: 100,
            master_volume: 100,
            audio_muted: false,
            fps_limit: 24,

            alarm_enabled: false,
            alarm_hour: 7,
            alarm_minute: 30,
            alarm_mode: 1,
            alarm_year,
            alarm_month,
            alarm_day,
            // Weekly alarms start with every day enabled.
            alarm_repeat_days: 0b0111_1111,

            alarm_book: "v1|".to_string(),
            alarm_book_initialized: false,
            alarm_sound: 1,
        }
    }
}

impl NeraSettings {
    pub(crate) fn throw_strength(&self) -> f32 {
        if self.throw_enabled {
            f32::from(self.throw_intensity.min(100)) / 100.0
        } else {
            0.0
        }
    }

    pub(super) fn throw_intensity_label(&self) -> String {
        format!("{}%", self.throw_intensity.min(100))
    }

    pub(crate) fn activity_level(&self) -> ActivityLevel {
        match self.activity_level {
            0 => ActivityLevel::Calm,
            2 => ActivityLevel::Active,
            _ => ActivityLevel::Normal,
        }
    }

    pub(crate) fn roaming_radius_px(&self) -> f32 {
        self.roaming_radius_px.clamp(300, 600) as f32
    }

    pub(super) fn cycle_activity_level(&mut self) {
        self.activity_level = match self.activity_level() {
            ActivityLevel::Calm => 1,
            ActivityLevel::Normal => 2,
            ActivityLevel::Active => 0,
        };
    }

    pub(crate) fn toggle_always_on_top(&mut self) {
        self.always_on_top = !self.always_on_top;
    }

    pub(crate) fn character_scale(&self) -> f32 {
        self.character_size.clamp(50, 150) as f32 / 100.0
    }

    pub(crate) fn ground_offset_logical(&self) -> f32 {
        self.ground_offset.clamp(-40, 40) as f32
    }

    pub(super) fn cycle_character_size(&mut self) {
        self.character_size = match self.character_size {
            50 => 75,
            75 => 100,
            100 => 125,
            125 => 150,
            _ => 50,
        };
    }

    pub(super) fn character_size_label(&self) -> String {
        format!("{}%", self.character_size.clamp(50, 150))
    }

    pub(super) fn cycle_ground_offset(&mut self) {
        self.ground_offset = match self.ground_offset {
            -40 => -20,
            -20 => 0,
            0 => 20,
            20 => 40,
            _ => -40,
        };
    }

    pub(super) fn ground_offset_label(&self) -> String {
        format!("{:+} px", self.ground_offset.clamp(-40, 40))
    }

    pub(super) fn cycle_follow_intensity(&mut self) {
        self.follow_intensity = match self.follow_intensity {
            25 => 50,
            50 => 75,
            75 => 100,
            _ => 25,
        };
    }

    pub(super) fn follow_intensity_label(&self) -> String {
        format!("{}%", self.follow_intensity.clamp(25, 100))
    }

    pub(super) fn cycle_shadow_type(&mut self) {
        self.shadow_type = if self.shadow_type == 0 { 1 } else { 0 };
    }

    pub(super) fn shadow_type_label(&self) -> &'static str {
        if self.shadow_type == 1 {
            "Character"
        } else {
            "Simple"
        }
    }

    pub(crate) fn uses_simple_shadow(&self) -> bool {
        self.shadow_type != 1
    }

    pub(crate) fn uses_character_shadow(&self) -> bool {
        self.shadow_type == 1
    }

    pub(super) fn cycle_shadow_opacity(&mut self) {
        self.shadow_opacity = match self.shadow_opacity {
            25 => 40,
            40 => 55,
            55 => 70,
            _ => 25,
        };
    }

    pub(super) fn shadow_opacity_label(&self) -> String {
        format!("{}%", self.shadow_opacity.clamp(0, 100))
    }

    pub(super) fn cycle_shadow_softness(&mut self) {
        self.shadow_softness = match self.shadow_softness {
            25 => 50,
            50 => 75,
            75 => 100,
            _ => 25,
        };
    }

    pub(super) fn shadow_softness_label(&self) -> String {
        format!("{}%", self.shadow_softness.clamp(25, 100))
    }

    pub(super) fn cycle_brightness(&mut self) {
        self.brightness = match self.brightness {
            75 => 100,
            100 => 125,
            125 => 150,
            _ => 75,
        };
    }

    pub(super) fn brightness_label(&self) -> String {
        format!("{}%", self.brightness.clamp(50, 200))
    }

    pub(super) fn cycle_master_volume(&mut self) {
        self.master_volume = match self.master_volume {
            25 => 50,
            50 => 75,
            75 => 100,
            _ => 25,
        };
    }

    pub(super) fn master_volume_label(&self) -> String {
        format!("{}%", self.master_volume.clamp(0, 100))
    }

    pub(super) fn audio_volume(&self) -> Volume {
        if self.audio_muted {
            Volume::SILENT
        } else {
            Volume::Linear(self.master_volume.clamp(0, 100) as f32 / 100.0)
        }
    }

    pub(super) fn cycle_fps_limit(&mut self) {
        self.fps_limit = match self.fps_limit {
            24 => 30,
            30 => 60,
            60 => 90,
            90 => 120,
            120 => 144,
            144 => 0,
            _ => 24,
        };
    }

    pub(super) fn fps_label(&self) -> String {
        match self.fps_limit {
            0 => "Unlimited (VSync)".to_string(),
            // Legacy 15 FPS values are retired and now behave as 24 FPS.
            15 => "24 FPS".to_string(),
            fps => format!("{fps} FPS"),
        }
    }

    pub(crate) fn alarm_mode_label(&self) -> &'static str {
        if self.alarm_mode == 0 {
            "One-time"
        } else {
            "Repeat weekly"
        }
    }

    pub(crate) fn alarm_date_label(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}",
            self.alarm_year,
            self.alarm_month.clamp(1, 12),
            self.alarm_day
                .clamp(1, alarm_days_in_month(self.alarm_year, self.alarm_month),),
        )
    }

    pub(crate) fn alarm_day_enabled(&self, bit: u8) -> bool {
        self.alarm_repeat_days & (1 << bit.min(6)) != 0
    }

    pub(crate) fn cycle_alarm_hour(&mut self) {
        self.alarm_hour = (self.alarm_hour + 1) % 24;
    }

    pub(crate) fn cycle_alarm_minute(&mut self) {
        if self.alarm_minute >= 59 {
            self.alarm_minute = 0;
            self.alarm_hour = (self.alarm_hour + 1) % 24;
        } else {
            self.alarm_minute += 1;
        }
    }

    pub(crate) fn cycle_alarm_mode(&mut self) {
        self.alarm_mode = if self.alarm_mode == 0 { 1 } else { 0 };
    }

    pub(crate) fn cycle_alarm_year(&mut self) {
        #[cfg(target_os = "windows")]
        let current_year = crate::platform::local_datetime().year;

        #[cfg(not(target_os = "windows"))]
        let current_year = 2026;

        self.alarm_year = if self.alarm_year < current_year
            || self.alarm_year >= current_year.saturating_add(5)
        {
            current_year
        } else {
            self.alarm_year.saturating_add(1)
        };

        self.alarm_day = self
            .alarm_day
            .min(alarm_days_in_month(self.alarm_year, self.alarm_month));
    }

    pub(crate) fn cycle_alarm_month(&mut self) {
        self.alarm_month = if self.alarm_month >= 12 {
            1
        } else {
            self.alarm_month.max(1) + 1
        };

        self.alarm_day = self
            .alarm_day
            .min(alarm_days_in_month(self.alarm_year, self.alarm_month));
    }

    pub(crate) fn cycle_alarm_day(&mut self) {
        let max_day = alarm_days_in_month(self.alarm_year, self.alarm_month);
        self.alarm_day = if self.alarm_day >= max_day {
            1
        } else {
            self.alarm_day.max(1) + 1
        };
    }

    pub(crate) fn toggle_alarm_day(&mut self, bit: u8) {
        self.alarm_repeat_days ^= 1 << bit.min(6);
    }
}

fn encode_label_hex(label: &str) -> String {
    let mut encoded = String::with_capacity(label.len() * 2);
    for byte in label.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02X}");
    }
    encoded
}

fn decode_label_hex(encoded: &str) -> Option<String> {
    if encoded.len() % 2 != 0 {
        return None;
    }

    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    let chars = encoded.as_bytes();

    for chunk in chars.chunks_exact(2) {
        let hi = (chunk[0] as char).to_digit(16)? as u8;
        let lo = (chunk[1] as char).to_digit(16)? as u8;
        bytes.push((hi << 4) | lo);
    }

    String::from_utf8(bytes).ok()
}

fn encode_alarm_book(alarms: &[AlarmConfig]) -> String {
    let mut encoded = String::from("v1|");

    for alarm in alarms {
        let alarm = alarm.clone().normalized();
        encoded.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{}|",
            alarm.id,
            u8::from(alarm.enabled),
            alarm.hour,
            alarm.minute,
            alarm.mode,
            alarm.year,
            alarm.month,
            alarm.day,
            alarm.repeat_days,
            encode_label_hex(&alarm.label),
            alarm.sound,
        ));
    }

    encoded
}

fn decode_alarm_book(book: &str) -> Option<Vec<AlarmConfig>> {
    let body = book.strip_prefix("v1|")?;
    let mut alarms = Vec::new();

    for record in body.split('|').filter(|record| !record.is_empty()) {
        let fields: Vec<_> = record.split(',').collect();
        if fields.len() != 10 && fields.len() != 11 {
            warn!("Settings: skipping malformed alarm record");
            continue;
        }

        let parsed = (|| {
            Some(
                AlarmConfig {
                    id: fields[0].parse().ok()?,
                    enabled: fields[1].parse::<u8>().ok()? != 0,
                    hour: fields[2].parse().ok()?,
                    minute: fields[3].parse().ok()?,
                    mode: fields[4].parse().ok()?,
                    year: fields[5].parse().ok()?,
                    month: fields[6].parse().ok()?,
                    day: fields[7].parse().ok()?,
                    repeat_days: fields[8].parse().ok()?,
                    label: decode_label_hex(fields[9])?,
                    sound: fields
                        .get(10)
                        .and_then(|sound| sound.parse().ok())
                        .unwrap_or(0),
                }
                .normalized(),
            )
        })();

        if let Some(alarm) = parsed {
            alarms.push(alarm);
        } else {
            warn!("Settings: skipping invalid alarm record");
        }
    }

    Some(alarms)
}

impl NeraSettings {
    /// Resolve the multi-alarm book. A missing pre-5.77 value is migrated from
    /// the old single-alarm fields so an existing custom alarm is not lost.
    pub(crate) fn alarm_configs(&self) -> Vec<AlarmConfig> {
        if self.alarm_book_initialized {
            return decode_alarm_book(&self.alarm_book).unwrap_or_default();
        }

        vec![
            AlarmConfig {
                id: 1,
                enabled: self.alarm_enabled,
                label: "Alarm".to_string(),
                hour: self.alarm_hour,
                minute: self.alarm_minute,
                mode: self.alarm_mode,
                year: self.alarm_year,
                month: self.alarm_month,
                day: self.alarm_day,
                repeat_days: self.alarm_repeat_days,
                sound: self.alarm_sound,
            }
            .normalized(),
        ]
    }

    pub(crate) fn store_alarm_configs(&mut self, alarms: &[AlarmConfig]) {
        self.alarm_book = encode_alarm_book(alarms);
        self.alarm_book_initialized = true;
    }

    pub(crate) fn next_alarm_id(&self) -> u32 {
        self.alarm_configs()
            .iter()
            .map(|alarm| alarm.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1)
    }
}

#[cfg(test)]
mod walking_settings_tests {
    use super::*;

    #[test]
    fn audio_reactive_preference_defaults_off_and_restores_through_existing_schema() {
        let mut settings = NeraSettings::default();
        assert!(!settings.audio_reactive_dance);
        let mut saved = bevy::reflect::structs::DynamicStruct::default();
        saved.insert("audio_reactive_dance", true);
        settings.apply(&saved);
        assert!(settings.audio_reactive_dance);
        saved.insert("audio_reactive_dance", false);
        settings.apply(&saved);
        assert!(!settings.audio_reactive_dance);
    }

    #[test]
    fn older_profiles_keep_walking_off_and_saved_choice_is_reflectable() {
        // SettingsPlugin starts from Default and applies only persisted fields.
        let mut legacy = bevy::reflect::structs::DynamicStruct::default();
        legacy.insert("character_size", 125_u16);
        legacy.insert("follow_mouse", false);
        let mut settings = NeraSettings::default();
        settings.apply(&legacy);
        assert_eq!(settings.character_size, 125);
        assert!(!settings.follow_mouse);
        assert!(!settings.walking_enabled);
        assert_eq!(settings.activity_level(), ActivityLevel::Normal);
        assert_eq!(settings.roaming_radius_px(), 400.0);
        assert!(settings.pause_roaming_fullscreen);

        let mut saved = bevy::reflect::structs::DynamicStruct::default();
        saved.insert("walking_enabled", true);
        settings.apply(&saved);
        assert!(settings.walking_enabled);
        assert_eq!(settings.character_size, 125);
    }
}

#[cfg(test)]
mod throw_settings_tests {
    use super::*;

    #[test]
    fn legacy_profiles_default_to_gentler_throws_and_saved_preferences_are_reflectable() {
        let mut old = bevy::reflect::structs::DynamicStruct::default();
        old.insert("gravity_enabled", false);
        let mut settings = NeraSettings::default();
        settings.apply(&old);
        assert!(settings.throw_enabled);
        assert_eq!(settings.throw_intensity, 50);
        assert_eq!(settings.throw_strength(), 0.5);
        assert!(!settings.gravity_enabled);

        let mut saved = bevy::reflect::structs::DynamicStruct::default();
        saved.insert("throw_enabled", false);
        saved.insert("throw_intensity", 30_u8);
        settings.apply(&saved);
        assert!(!settings.throw_enabled);
        assert_eq!(settings.throw_intensity, 30);
        assert_eq!(settings.throw_strength(), 0.0);
        settings.throw_enabled = true;
        assert_eq!(settings.throw_strength(), 0.3);
        settings.throw_intensity = u8::MAX;
        assert_eq!(settings.throw_strength(), 1.0);
        settings.throw_intensity = 0;
        assert_eq!(settings.throw_strength(), 0.0);
    }
}

#[cfg(test)]
mod roaming_settings_tests {
    use super::*;

    #[test]
    fn malformed_roaming_settings_use_safe_runtime_values() {
        let mut settings = NeraSettings::default();
        settings.activity_level = u8::MAX;
        settings.roaming_radius_px = 0;
        assert_eq!(settings.activity_level(), ActivityLevel::Normal);
        assert_eq!(settings.roaming_radius_px(), 300.0);
        settings.roaming_radius_px = u16::MAX;
        assert_eq!(settings.roaming_radius_px(), 600.0);
    }

    #[test]
    fn activity_choices_follow_requested_cadence() {
        let mut settings = NeraSettings::default();
        assert_eq!(settings.activity_level().walk_interval_secs(), (30.0, 90.0));
        settings.cycle_activity_level();
        assert_eq!(settings.activity_level().walk_interval_secs(), (15.0, 45.0));
        settings.cycle_activity_level();
        assert_eq!(
            settings.activity_level().walk_interval_secs(),
            (60.0, 180.0)
        );
        settings.cycle_activity_level();
        assert_eq!(settings.activity_level(), ActivityLevel::Normal);
    }

    #[test]
    fn saved_roaming_preferences_are_reflectable() {
        let mut saved = bevy::reflect::structs::DynamicStruct::default();
        saved.insert("activity_level", 2_u8);
        saved.insert("roaming_radius_px", 550_u16);
        saved.insert("pause_roaming_fullscreen", false);
        let mut settings = NeraSettings::default();
        settings.apply(&saved);
        assert_eq!(settings.activity_level(), ActivityLevel::Active);
        assert_eq!(settings.roaming_radius_px(), 550.0);
        assert!(!settings.pause_roaming_fullscreen);
    }
}

#[cfg(test)]
mod alarm_book_tests {
    use super::*;

    #[test]
    fn imported_and_random_sound_ids_survive_alarm_book_normalization() {
        for sound in [5, 1024, crate::alarm::sounds::RANDOM_SOUND] {
            let mut alarm = AlarmConfig::default_new(17);
            alarm.sound = sound;
            assert_eq!(
                decode_alarm_book(&encode_alarm_book(&[alarm.clone()])).unwrap(),
                vec![alarm]
            );
        }
    }

    #[test]
    fn old_alarm_records_keep_classic_sound_and_new_records_preserve_choices() {
        let old = decode_alarm_book("v1|3,1,6,45,1,2026,9,23,31,57616b65207570|").unwrap();
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].sound, 0);
        let mut alarm = old[0].clone();
        alarm.sound = 4;
        assert_eq!(
            decode_alarm_book(&encode_alarm_book(&[alarm.clone()])).unwrap(),
            vec![alarm]
        );
    }

    #[test]
    fn alarm_book_roundtrip_preserves_multiple_entries_and_labels() {
        let alarms = vec![
            AlarmConfig {
                id: 3,
                enabled: true,
                label: "Wake up".into(),
                hour: 6,
                minute: 45,
                mode: 1,
                year: 2026,
                month: 9,
                day: 23,
                repeat_days: 0b0001_1111,
                sound: 2,
            },
            AlarmConfig {
                id: 4,
                enabled: false,
                label: "Meeting, A|B".into(),
                hour: 14,
                minute: 5,
                mode: 0,
                year: 2026,
                month: 10,
                day: 2,
                repeat_days: 0,
                sound: 4,
            },
        ];

        let encoded = encode_alarm_book(&alarms);
        let decoded = decode_alarm_book(&encoded).expect("valid v1 alarm book");

        assert_eq!(decoded, alarms);
    }

    #[test]
    fn storing_empty_alarm_book_does_not_resurrect_legacy_alarm() {
        let mut settings = NeraSettings::default();
        settings.alarm_enabled = true;
        settings.store_alarm_configs(&[]);

        assert!(settings.alarm_book_initialized);
        assert!(settings.alarm_configs().is_empty());
    }

    #[test]
    fn pre_multi_alarm_profile_exposes_legacy_alarm_for_migration() {
        let mut settings = NeraSettings::default();
        settings.alarm_book_initialized = false;
        settings.alarm_enabled = true;
        settings.alarm_hour = 8;
        settings.alarm_minute = 15;

        let alarms = settings.alarm_configs();
        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].hour, 8);
        assert_eq!(alarms[0].minute, 15);
        assert!(alarms[0].enabled);
    }
    #[test]
    fn weekly_alarm_with_no_days_normalizes_to_off() {
        let mut alarm = AlarmConfig::default_new(99);
        alarm.mode = 1;
        alarm.repeat_days = 0;
        alarm.enabled = true;

        let alarm = alarm.normalized();
        assert!(!alarm.enabled);
    }
}

#[cfg(test)]
mod body_mode_settings_tests {
    use super::*;

    #[test]
    fn legacy_profiles_default_to_normal_and_saved_chibi_is_reflectable() {
        let mut legacy = bevy::reflect::structs::DynamicStruct::default();
        legacy.insert("character_size", 125_u16);
        let mut settings = NeraSettings::default();
        settings.apply(&legacy);
        assert_eq!(settings.body_mode, "normal");
        assert_eq!(settings.character_size, 125);

        let mut saved = bevy::reflect::structs::DynamicStruct::default();
        saved.insert("body_mode", "chibi".to_string());
        settings.apply(&saved);
        assert_eq!(settings.body_mode, "chibi");
        assert_eq!(settings.character_size, 125);
    }
}

#[cfg(test)]
mod gaze_settings_tests {
    use super::*;

    #[test]
    fn default_gaze_settings_enable_head_follow_with_quarter_intensity() {
        let settings = NeraSettings::default();
        assert!(settings.follow_mouse);
        assert!(settings.follow_eyes);
        assert!(settings.follow_head);
        assert_eq!(settings.follow_intensity, 25);
        assert_eq!(settings.follow_intensity_label(), "25%");
    }
}
