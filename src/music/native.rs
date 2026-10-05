//! Bounded latest-value mailboxes. All COM/WinRT and DSP run off the Bevy thread.
use super::{AudioEvidence, MediaEvidence, MusicDetectionState, dsp::Analyzer};
use bevy::prelude::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows::{
    Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager as SessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
    },
    Win32::{
        Media::Audio::*,
        System::{Com::*, WinRT::*},
    },
    core::{GUID, Result},
};

#[derive(Default)]
struct Latest {
    media: MediaEvidence,
    audio: AudioEvidence,
    media_at: Option<Instant>,
    audio_at: Option<Instant>,
}
struct Control {
    alive: AtomicBool,
    enabled: AtomicBool,
}
#[derive(Resource)]
pub(super) struct MusicWorker {
    control: Arc<Control>,
    latest: Arc<Mutex<Latest>>,
    started: bool,
    poll_in: f32,
    diagnostic_at: f64,
    diagnostic_key: Option<(String, bool, super::ConfidenceTier)>,
}
impl Default for MusicWorker {
    fn default() -> Self {
        Self {
            control: Arc::new(Control {
                alive: AtomicBool::new(true),
                enabled: AtomicBool::new(false),
            }),
            latest: Arc::new(Mutex::new(Latest::default())),
            started: false,
            poll_in: 0.0,
            diagnostic_at: 0.0,
            diagnostic_key: None,
        }
    }
}
impl Drop for MusicWorker {
    fn drop(&mut self) {
        self.control.alive.store(false, Ordering::Relaxed);
    }
}
impl Control {
    fn enabled(&self) -> bool {
        self.alive.load(Ordering::Relaxed) && self.enabled.load(Ordering::Relaxed)
    }
}

pub(super) fn poll(
    settings: Res<crate::settings::NeraSettings>,
    time: Res<Time<Real>>,
    mut worker: ResMut<MusicWorker>,
    mut state: ResMut<MusicDetectionState>,
) {
    let enabled = settings.audio_reactive_dance;
    worker.control.enabled.store(enabled, Ordering::Relaxed);
    if !enabled {
        *state = MusicDetectionState::default();
        if let Ok(mut latest) = worker.latest.try_lock() {
            *latest = Latest::default();
        }
        worker.poll_in = 0.0;
        worker.diagnostic_key = None;
        return;
    }
    if !worker.started {
        worker.started = true;
        for (name, run) in [
            (
                "nera-media",
                media_worker as fn(Arc<Control>, Arc<Mutex<Latest>>),
            ),
            ("nera-loopback", audio_worker),
        ] {
            let (control, latest) = (worker.control.clone(), worker.latest.clone());
            if let Err(error) = thread::Builder::new()
                .name(name.into())
                .spawn(move || run(control, latest))
            {
                warn!("Music detector thread unavailable: {error}");
            }
        }
    }
    worker.poll_in -= time.delta_secs();
    if worker.poll_in > 0.0 {
        return;
    }
    worker.poll_in = 0.25;
    let mailbox = worker.latest.clone();
    if let Ok(latest) = mailbox.try_lock() {
        let media = if latest
            .media_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(3))
        {
            latest.media.clone()
        } else {
            MediaEvidence::default()
        };
        let audio = if latest
            .audio_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            latest.audio
        } else {
            AudioEvidence::default()
        };
        // Suspended rendering cannot contribute unobserved seconds to hysteresis.
        let previous = state.active;
        state.observe(&media, audio, 0.25);
        if previous != state.active {
            info!(
                "{} confidence={:.2} tier={:?}",
                if state.active {
                    "MusicDetected"
                } else {
                    "MusicStopped"
                },
                state.confidence,
                state.tier
            );
        }
        let key = (media.source.clone(), media.playback_type_music, state.tier);
        let now = time.elapsed_secs_f64();
        // Log evidence changes at most once per two seconds; optional native
        // diagnostics also emit a five-second heartbeat for stable evidence.
        let changed = worker.diagnostic_key.as_ref() != Some(&key);
        let heartbeat = std::env::var_os("NERA_MUSIC_DIAGNOSTICS").is_some();
        if (changed && now >= worker.diagnostic_at + 2.0)
            || (heartbeat && now >= worker.diagnostic_at + 5.0)
        {
            info!(
                "Music evidence source={} playing={} PlaybackType::Music={} metadata(title/artist/album)={}/{}/{} media_score={:.2} audio_active={} audio_probability={:.2} confidence={:.2} tier={:?}",
                media.source,
                media.playing,
                media.playback_type_music,
                !media.title.is_empty(),
                !media.artist.is_empty(),
                !media.album.is_empty(),
                media.confidence,
                audio.active,
                audio.probability,
                state.confidence,
                state.tier
            );
            // Release the mailbox read lock before updating the worker resource.
            drop(latest);
            worker.diagnostic_at = now;
            worker.diagnostic_key = Some(key);
        }
    }
}

fn wait_ready(control: &Control, ready: impl Fn() -> Result<bool>) -> Result<()> {
    let start = Instant::now();
    while !ready()? {
        if !control.enabled() || start.elapsed() > Duration::from_millis(750) {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x800705B4u32 as i32,
            )));
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
fn media_worker(control: Arc<Control>, latest: Arc<Mutex<Latest>>) {
    if unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_err() {
        return;
    }
    let mut manager = None;
    let mut warned = false;
    while control.alive.load(Ordering::Relaxed) {
        if !control.enabled() {
            manager = None;
            thread::sleep(Duration::from_millis(100));
            continue;
        }
        let result = (|| -> Result<MediaEvidence> {
            if manager.is_none() {
                let op = SessionManager::RequestAsync()?;
                wait_ready(&control, || op.Status().map(|s| s.0 != 0))?;
                manager = Some(op.GetResults()?);
            }
            let sessions = manager.as_ref().unwrap().GetSessions()?;
            let mut best = MediaEvidence {
                available: true,
                ..Default::default()
            };
            for index in 0..sessions.Size()?.min(8) {
                let session = sessions.GetAt(index)?;
                let playback = session.GetPlaybackInfo()?;
                let playing = playback.PlaybackStatus()? == PlaybackStatus::Playing;
                if !playing {
                    continue;
                }
                let source = session.SourceAppUserModelId()?.to_string();
                let playback_music = playback
                    .PlaybackType()
                    .and_then(|t| t.Value())
                    .is_ok_and(|t| t == windows::Media::MediaPlaybackType::Music);
                let mut evidence =
                    MediaEvidence::classify(&source, true, "", "", "", playback_music);
                // Missing/timed-out metadata must not discard a useful playing
                // session, especially a dedicated player with partial metadata.
                if let Ok(op) = session.TryGetMediaPropertiesAsync()
                    && wait_ready(&control, || op.Status().map(|s| s.0 != 0)).is_ok()
                    && let Ok(properties) = op.GetResults()
                {
                    let title = properties
                        .Title()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let artist = properties
                        .Artist()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let album = properties
                        .AlbumTitle()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let audio_type = playback_music
                        || properties
                            .PlaybackType()
                            .and_then(|t| t.Value())
                            .is_ok_and(|t| t == windows::Media::MediaPlaybackType::Music);
                    evidence = MediaEvidence::classify(
                        &source, playing, &title, &artist, &album, audio_type,
                    );
                }
                if evidence.confidence > best.confidence {
                    best = evidence;
                }
            }
            Ok(best)
        })();
        let evidence = match result {
            Ok(value) => {
                warned = false;
                value
            }
            Err(error) => {
                if !warned && control.enabled() {
                    warn!("Media Session detection unavailable: {error}");
                    warned = true;
                }
                manager = None;
                MediaEvidence::default()
            }
        };
        if let Ok(mut value) = latest.lock() {
            value.media = evidence;
            value.media_at = Some(Instant::now());
        }
        for _ in 0..10 {
            if !control.enabled() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    drop(manager);
    unsafe {
        RoUninitialize();
    }
}

struct Loopback {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    enumerator: IMMDeviceEnumerator,
    endpoint: String,
    rate: u32,
    channels: usize,
    stride: usize,
    bytes: usize,
    float: bool,
}
impl Drop for Loopback {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}
fn endpoint(enumerator: &IMMDeviceEnumerator) -> Result<(IMMDevice, String)> {
    unsafe {
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let id = device.GetId()?;
        let name = id.to_string();
        CoTaskMemFree(Some(id.0.cast()));
        Ok((device, name?))
    }
}
impl Loopback {
    fn open() -> Result<Self> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let (device, endpoint) = endpoint(&enumerator)?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let format = client.GetMixFormat()?;
            let fmt = *format;
            let tag = fmt.wFormatTag;
            let subtype = if tag == 0xfffe && fmt.cbSize >= 22 {
                (*(format as *const WAVEFORMATEXTENSIBLE)).SubFormat
            } else {
                GUID::zeroed()
            };
            let float =
                tag == 3 || subtype == GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
            let pcm =
                tag == 1 || subtype == GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
            let bytes = fmt.wBitsPerSample as usize / 8;
            let supported = (float && bytes == 4) || (pcm && [2, 3, 4].contains(&bytes));
            let (rate, channels) = (fmt.nSamplesPerSec, fmt.nChannels);
            let valid = supported
                && (8000..=384000).contains(&rate)
                && (1..=32).contains(&channels)
                && fmt.nBlockAlign as usize >= bytes * channels as usize;
            let initialized = if valid {
                client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK,
                    1_000_000,
                    0,
                    format,
                    None,
                )
            } else {
                Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x80070057u32 as i32,
                )))
            };
            CoTaskMemFree(Some(format.cast()));
            initialized?;
            let capture = client.GetService::<IAudioCaptureClient>()?;
            client.Start()?;
            Ok(Self {
                client,
                capture,
                enumerator,
                endpoint,
                rate: fmt.nSamplesPerSec,
                channels: fmt.nChannels as usize,
                stride: fmt.nBlockAlign as usize,
                bytes,
                float,
            })
        }
    }
    fn read(&self, analyzer: &mut Analyzer) -> Result<bool> {
        let mut received = false;
        let mut remaining_frames = self.rate / 2;
        // Bound packet draining even if a buggy device never reports empty.
        for _ in 0..64 {
            unsafe {
                if self.capture.GetNextPacketSize()? == 0 {
                    break;
                }
                let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0, 0);
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                    *analyzer = Analyzer::default();
                }
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null();
                let analyzed_frames = frames.min(remaining_frames);
                for frame in 0..analyzed_frames as usize {
                    let mut mono = 0.0;
                    if !silent {
                        for channel in 0..self.channels {
                            let pointer = data.add(frame * self.stride + channel * self.bytes);
                            mono += if self.float {
                                pointer.cast::<f32>().read_unaligned()
                            } else if self.bytes == 2 {
                                pointer.cast::<i16>().read_unaligned() as f32 / 32768.0
                            } else if self.bytes == 3 {
                                let v = (*pointer as i32)
                                    | ((*pointer.add(1) as i32) << 8)
                                    | ((*pointer.add(2) as i32) << 16);
                                ((v << 8) >> 8) as f32 / 8388608.0
                            } else {
                                pointer.cast::<i32>().read_unaligned() as f32 / 2147483648.0
                            };
                        }
                    }
                    let sample = mono / self.channels as f32;
                    analyzer.push(if sample.is_finite() { sample } else { 0.0 }, self.rate);
                }
                self.capture.ReleaseBuffer(frames)?;
                received = true;
                remaining_frames -= analyzed_frames;
                if remaining_frames == 0 {
                    break;
                }
            }
        }
        Ok(received)
    }
}

fn audio_worker(control: Arc<Control>, latest: Arc<Mutex<Latest>>) {
    if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
        return;
    }
    let (mut loopback, mut analyzer, mut retry, mut device_check) =
        (None, Analyzer::default(), Instant::now(), Instant::now());
    let mut warned = false;
    while control.alive.load(Ordering::Relaxed) {
        if !control.enabled() {
            loopback = None;
            analyzer = Analyzer::default();
            retry = Instant::now();
            thread::sleep(Duration::from_millis(100));
            continue;
        }
        if loopback.is_none() && Instant::now() >= retry {
            match Loopback::open() {
                Ok(value) => {
                    loopback = Some(value);
                    analyzer = Analyzer::default();
                    warned = false;
                }
                Err(error) => {
                    if !warned {
                        warn!("WASAPI output detection unavailable: {error}");
                        warned = true;
                    }
                }
            }
            retry = Instant::now() + Duration::from_secs(5);
        }
        let mut audio = AudioEvidence::default();
        if let Some(capture) = &loopback {
            let changed = device_check.elapsed() >= Duration::from_secs(2) && {
                device_check = Instant::now();
                endpoint(&capture.enumerator).is_ok_and(|(_, id)| id != capture.endpoint)
            };
            match capture.read(&mut analyzer) {
                Ok(received) if !changed => {
                    // WASAPI emits no packets when the endpoint goes silent.
                    if !received {
                        for _ in 0..400 {
                            analyzer.push(0.0, 8000);
                        }
                    }
                    audio = analyzer.evidence();
                }
                _ => {
                    loopback = None;
                    analyzer = Analyzer::default();
                    retry = Instant::now() + Duration::from_secs(1);
                }
            }
        }
        if let Ok(mut value) = latest.lock() {
            value.audio = audio;
            value.audio_at = Some(Instant::now());
        }
        thread::sleep(Duration::from_millis(50));
    }
    drop(loopback);
    unsafe {
        CoUninitialize();
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    struct Probe {
        control: Arc<Control>,
        latest: Arc<Mutex<Latest>>,
        handles: Vec<thread::JoinHandle<()>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.control.alive.store(false, Ordering::Relaxed);
            for handle in self.handles.drain(..) {
                let _ = handle.join();
            }
        }
    }
    fn output_probe() -> Probe {
        let control = Arc::new(Control {
            alive: AtomicBool::new(true),
            enabled: AtomicBool::new(true),
        });
        let latest = Arc::new(Mutex::new(Latest::default()));
        let (c, l) = (control.clone(), latest.clone());
        Probe {
            control,
            latest,
            handles: vec![thread::spawn(move || audio_worker(c, l))],
        }
    }
    #[test]
    #[ignore = "Plays a quiet generated validation tone; requires an otherwise silent Windows output"]
    fn native_loopback_music_start_stop() {
        let probe = output_probe();
        thread::sleep(Duration::from_secs(2));
        {
            let latest = probe.latest.lock().unwrap();
            assert!(latest.audio.available, "WASAPI unavailable");
            assert!(
                !latest.audio.active,
                "Close other playback before this opt-in silence/music/stop fixture"
            );
        }
        let mut output = rodio::DeviceSinkBuilder::open_default_sink().unwrap();
        output.log_on_drop(false);
        // Generated fixture only: no user audio recorded, saved or transmitted.
        let samples: Vec<f32> = (0..72000)
            .map(|index| {
                let t = index as f32 / 8000.0;
                let beat = 0.15 + 0.85 * (-((t * 2.0).fract() * 8.0)).exp();
                beat * 0.04
                    * ((t * 220.0 * std::f32::consts::TAU).sin()
                        + 0.5 * (t * 440.0 * std::f32::consts::TAU).sin()
                        + 0.3 * (t * 880.0 * std::f32::consts::TAU).sin())
            })
            .collect();
        output.mixer().add(rodio::buffer::SamplesBuffer::new(
            std::num::NonZeroU16::new(1).unwrap(),
            std::num::NonZeroU32::new(8000).unwrap(),
            samples,
        ));
        let mut state = MusicDetectionState::default();
        let mut detected = false;
        for _ in 0..32 {
            thread::sleep(Duration::from_millis(250));
            let value = probe.latest.lock().unwrap();
            state.observe(&MediaEvidence::default(), value.audio, 0.25);
            detected |= state.active;
            eprintln!(
                "LOOPBACK_FIXTURE audio={:?} music={} confidence={:.3}",
                value.audio, state.active, state.confidence
            );
        }
        assert!(
            detected,
            "Generated rhythmic music was not detected through actual Windows output"
        );
        drop(output);
        for _ in 0..24 {
            thread::sleep(Duration::from_millis(250));
            let value = probe.latest.lock().unwrap();
            state.observe(&MediaEvidence::default(), value.audio, 0.25);
        }
        assert!(
            !state.active,
            "Music ownership did not stop after output silence"
        );
        eprintln!(
            "LOOPBACK_FIXTURE_PASS silence -> generated quiet music -> stopped; no Media Session metadata"
        );
    }
    #[test]
    #[ignore = "Reads live Windows output and Media Sessions for eight seconds; opt in locally"]
    fn native_music_probe() {
        let pool = crate::dance_pool::DancePool::load(&crate::dance_pool::DancePool::file());
        eprintln!(
            "MUSIC_PROBE detection_enabled=true persisted_DancePool_count={} keys={:?}",
            pool.keys.len(),
            pool.keys
        );
        let control = Arc::new(Control {
            alive: AtomicBool::new(true),
            enabled: AtomicBool::new(true),
        });
        let latest = Arc::new(Mutex::new(Latest::default()));
        let mut handles = Vec::new();
        for run in [
            media_worker as fn(Arc<Control>, Arc<Mutex<Latest>>),
            audio_worker,
        ] {
            let (c, l) = (control.clone(), latest.clone());
            handles.push(thread::spawn(move || run(c, l)));
        }
        let mut state = MusicDetectionState::default();
        for _ in 0..32 {
            thread::sleep(Duration::from_millis(250));
            let value = latest.lock().unwrap();
            state.observe(&value.media, value.audio, 0.25);
            eprintln!(
                "MUSIC_PROBE media={:?} audio={:?} music={} confidence={:.3} tier={:?}",
                value.media, value.audio, state.active, state.confidence, state.tier
            );
        }
        control.alive.store(false, Ordering::Relaxed);
        for handle in handles {
            handle.join().unwrap();
        }
        assert!(
            state.media_available || state.audio_available,
            "Neither native API available"
        );
    }
}
