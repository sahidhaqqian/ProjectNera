//! Bounded 4-second analysis at 8 kHz. No recordings, dependencies or ML.
use super::AudioEvidence;
use std::collections::VecDeque;
const RATE: f32 = 8000.0;
const FRAME: usize = 256;
const HISTORY: usize = 125;

#[derive(Default)]
pub(super) struct Analyzer {
    samples: Vec<f32>,
    window: Vec<f32>,
    energy: VecDeque<f32>,
    onset: VecDeque<f32>,
    previous: [f32; 12],
    tonal: VecDeque<f32>,
    broad: VecDeque<f32>,
    phase: f32,
    low_pass: f32,
    source_rate: u32,
    alpha: f32,
}

impl Analyzer {
    pub(super) fn silence(&mut self, source_rate: u32, duration: std::time::Duration) {
        let frames = (duration.as_secs_f64() * f64::from(source_rate)).round() as usize;
        for _ in 0..frames {
            self.push(0.0, source_rate);
        }
    }
    pub(super) fn push(&mut self, sample: f32, source_rate: u32) {
        if source_rate < 8000 {
            return;
        }
        // Cheap low-pass before decimation to limit aliased high-frequency SFX.
        if self.source_rate != source_rate {
            self.source_rate = source_rate;
            self.phase = 0.0;
            self.alpha = (1.0 - (-2.0 * std::f32::consts::PI * 3000.0 / source_rate as f32).exp())
                .clamp(0.0, 1.0);
        }
        self.low_pass += self.alpha * (sample.clamp(-1.0, 1.0) - self.low_pass);
        self.phase += RATE;
        if self.phase < source_rate as f32 {
            return;
        }
        self.phase -= source_rate as f32;
        self.samples.push(self.low_pass);
        if self.samples.len() < FRAME {
            return;
        }
        if self.window.is_empty() {
            self.window = (0..FRAME)
                .map(|n| {
                    0.5 - 0.5 * (2.0 * std::f32::consts::PI * n as f32 / (FRAME - 1) as f32).cos()
                })
                .collect();
        }
        let rms = (self.samples.iter().map(|s| s * s).sum::<f32>() / FRAME as f32).sqrt();
        // Log-spaced Goertzel bands approximate spectral distribution and flux.
        let mut spectrum = [0.0; 12];
        for (band, frequency) in [
            80.0, 125.0, 196.0, 250.0, 330.0, 440.0, 660.0, 880.0, 1320.0, 1760.0, 2400.0, 3200.0,
        ]
        .iter()
        .enumerate()
        {
            let coefficient = 2.0 * (2.0 * std::f32::consts::PI * frequency / RATE).cos();
            let (mut a, mut b) = (0.0, 0.0);
            for (&s, &window) in self.samples.iter().zip(&self.window) {
                let next = s * window + coefficient * a - b;
                b = a;
                a = next;
            }
            spectrum[band] = (a * a + b * b - coefficient * a * b).max(0.0).sqrt() / FRAME as f32;
        }
        let total = spectrum.iter().sum::<f32>().max(1e-6);
        let peak = spectrum.iter().copied().fold(0.0, f32::max) / total;
        let covered = spectrum.iter().filter(|&&v| v / total > 0.04).count() as f32 / 12.0;
        let flux = spectrum
            .iter()
            .zip(self.previous)
            .map(|(a, b)| (a - b).max(0.0))
            .sum::<f32>();
        self.previous = spectrum;
        for (history, value) in [
            (&mut self.energy, rms),
            (&mut self.onset, flux),
            (&mut self.tonal, peak),
            (&mut self.broad, covered),
        ] {
            if history.len() == HISTORY {
                history.pop_front();
            }
            history.push_back(value);
        }
        self.samples.clear();
    }

    pub(super) fn evidence(&self) -> AudioEvidence {
        let recent = self.energy.iter().rev().take(8).copied().sum::<f32>() / 8.0;
        let active = recent > 0.001; // roughly -60 dBFS; quiet music remains eligible
        if !active || self.energy.len() < 64 {
            return AudioEvidence {
                available: true,
                active,
                probability: 0.0,
            };
        }
        let rhythm = periodicity(&self.onset).max(periodicity(&self.energy));
        let coverage =
            self.energy.iter().filter(|&&v| v > 0.001).count() as f32 / self.energy.len() as f32;
        let tonal =
            (self.tonal.iter().sum::<f32>() / self.tonal.len() as f32 * 2.5).clamp(0.0, 1.0);
        let broad =
            (self.broad.iter().sum::<f32>() / self.broad.len() as f32 * 1.8).clamp(0.0, 1.0);
        // Sustained rhythm is mandatory for metadata-free/browser detection.
        // Constant single tones, speech bursts and broadband noise score low.
        let probability = (0.70 * rhythm + 0.15 * tonal + 0.15 * broad) * coverage;
        AudioEvidence {
            available: true,
            active,
            probability: probability.clamp(0.0, 1.0),
        }
    }
}

#[cfg(test)]
mod silence_tests {
    use super::*;
    #[test]
    fn packet_gap_preserves_source_rate_and_decimation_phase() {
        for rate in [44_100, 48_000] {
            let mut analyzer = Analyzer::default();
            analyzer.push(0.25, rate);
            let alpha = analyzer.alpha;
            let phase = analyzer.phase;
            analyzer.silence(rate, std::time::Duration::from_millis(50));
            assert_eq!(analyzer.source_rate, rate);
            assert_eq!(analyzer.alpha, alpha);
            assert_eq!(analyzer.phase, phase);
            assert_eq!(analyzer.energy.len(), 1);
            assert_eq!(analyzer.samples.len(), 144);
        }
    }
}

fn periodicity(history: &VecDeque<f32>) -> f32 {
    if history.len() < 64 {
        return 0.0;
    }
    let mean = history.iter().sum::<f32>() / history.len() as f32;
    let variance = history.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / history.len() as f32;
    if variance < 1e-8 || variance.sqrt() < mean.abs() * 0.05 {
        return 0.0;
    }
    // 0.25–1.0s rhythmic periods; no BPM inference/export.
    (8..=31)
        .map(|lag| {
            let (mut dot, mut left, mut right) = (0.0, 0.0, 0.0);
            for i in lag..history.len() {
                let a = history[i] - mean;
                let b = history[i - lag] - mean;
                dot += a * b;
                left += a * a;
                right += b * b;
            }
            (dot / (left * right).sqrt().max(1e-9)).max(0.0)
        })
        .fold(0.0, f32::max)
}
