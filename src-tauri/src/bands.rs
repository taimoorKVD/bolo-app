//! Four-band loudness for the listening character's mouth.
//!
//! The capture thread already measures RMS. This adds a cheap bank of
//! band-pass filters so the overlay can shape the mouth from the real
//! microphone, not from a single loudness number.
//!
//! Bands match the v4 character mockup: 80–300, 300–1100, 1100–3200,
//! 3200–8000 Hz. Each value is 0..1.

use std::sync::atomic::{AtomicU32, Ordering};

const CENTERS: [f32; 4] = [180.0, 700.0, 2000.0, 5200.0];
const Q: f32 = 0.85;
/// A full-scale band-pass sine lands near 1.0. Quiet rooms stay near 0.
const GAIN: f32 = 7.5;

#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn silent() -> Self {
        Self { b0: 0.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    fn bandpass(freq: f32, rate: f32) -> Self {
        if !(freq > 0.0) || freq >= rate * 0.45 || rate < 1000.0 {
            return Self::silent();
        }
        let w0 = std::f32::consts::TAU * freq / rate;
        let alpha = w0.sin() / (2.0 * Q);
        let a0 = 1.0 + alpha;
        Self {
            b0: alpha / a0,
            b1: 0.0,
            b2: -alpha / a0,
            a1: (-2.0 * w0.cos()) / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

pub struct BandMeter {
    rate: u32,
    filters: [Biquad; 4],
    smooth: [f32; 4],
}

impl BandMeter {
    pub fn new() -> Self {
        Self { rate: 0, filters: [Biquad::silent(); 4], smooth: [0.0; 4] }
    }

    pub fn set_rate(&mut self, rate: u32) {
        if rate == self.rate || rate < 1000 {
            return;
        }
        self.rate = rate;
        let hz = rate as f32;
        self.filters = [
            Biquad::bandpass(CENTERS[0], hz),
            Biquad::bandpass(CENTERS[1], hz),
            Biquad::bandpass(CENTERS[2], hz),
            Biquad::bandpass(CENTERS[3], hz),
        ];
        self.smooth = [0.0; 4];
    }

    /// Mix interleaved frames to mono, update the smoothed 0..1 bands, and
    /// return them. `channels` is 1 for the usual capture format.
    pub fn push(&mut self, samples: &[f32], channels: u16) -> [f32; 4] {
        if self.rate < 1000 {
            self.set_rate(16_000);
        }
        let ch = usize::from(channels.max(1));
        let mut acc = [0.0f32; 4];
        let mut n = 0u32;
        if ch == 1 {
            for &sample in samples {
                self.tick(sample, &mut acc);
                n += 1;
            }
        } else {
            for frame in samples.chunks(ch) {
                if frame.is_empty() {
                    continue;
                }
                let mixed = frame.iter().sum::<f32>() / frame.len() as f32;
                self.tick(mixed, &mut acc);
                n += 1;
            }
        }
        if n == 0 {
            return self.smooth;
        }
        let inv = 1.0 / n as f32;
        for i in 0..4 {
            let rms = (acc[i] * inv).sqrt();
            let norm = (rms * GAIN).clamp(0.0, 1.0);
            let follow = if norm > self.smooth[i] { 0.62 } else { 0.28 };
            self.smooth[i] += (norm - self.smooth[i]) * follow;
        }
        self.smooth
    }

    pub fn publish(&mut self, samples: &[f32], channels: u16, out: &[AtomicU32; 4]) {
        let values = self.push(samples, channels);
        for (slot, value) in out.iter().zip(values) {
            slot.store(value.to_bits(), Ordering::Relaxed);
        }
    }

    fn tick(&mut self, sample: f32, acc: &mut [f32; 4]) {
        for (filter, bucket) in self.filters.iter_mut().zip(acc.iter_mut()) {
            let y = filter.process(sample);
            *bucket += y * y;
        }
    }
}

impl Default for BandMeter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn blank_bands() -> [AtomicU32; 4] {
    [
        AtomicU32::new(0),
        AtomicU32::new(0),
        AtomicU32::new(0),
        AtomicU32::new(0),
    ]
}

pub fn read_bands(bands: &[AtomicU32; 4]) -> [f32; 4] {
    [0, 1, 2, 3].map(|i| {
        let value = f32::from_bits(bands[i].load(Ordering::Relaxed));
        if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, rate: u32, secs: f32, amp: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                (t * freq * std::f32::consts::TAU).sin() * amp
            })
            .collect()
    }

    #[test]
    fn silence_stays_closed() {
        let mut meter = BandMeter::new();
        meter.set_rate(16_000);
        let bands = meter.push(&[0.0; 8_000], 1);
        assert!(bands.iter().all(|band| *band < 0.05), "{bands:?}");
    }

    #[test]
    fn a_vowel_lights_its_own_band() {
        let mut meter = BandMeter::new();
        meter.set_rate(16_000);
        let bands = meter.push(&tone(700.0, 16_000, 0.35, 0.22), 1);
        assert!(bands[1] > 0.45, "mid band should open, got {bands:?}");
        assert!(bands[1] > bands[0] * 1.4, "700 Hz is not the low band: {bands:?}");
        assert!(bands[1] > bands[3], "700 Hz is not the hiss band: {bands:?}");
    }

    #[test]
    fn a_low_tone_and_a_hiss_pick_different_bands() {
        let mut low = BandMeter::new();
        low.set_rate(16_000);
        let low_bands = low.push(&tone(180.0, 16_000, 0.35, 0.22), 1);
        let mut hiss = BandMeter::new();
        hiss.set_rate(16_000);
        let hiss_bands = hiss.push(&tone(5200.0, 16_000, 0.35, 0.22), 1);
        assert!(low_bands[0] > low_bands[3], "low tone: {low_bands:?}");
        assert!(hiss_bands[3] > hiss_bands[0], "hiss: {hiss_bands:?}");
    }
}
