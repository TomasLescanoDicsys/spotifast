//! The sound's modulations and filtering, measured the way a
//! communications engineer reads a signal. Music has no carrier, but it
//! does modulate:
//!
//! - AM: the envelope, an RMS every 10 ms, swings at the beat or at a
//!   tremolo's rate. Its strongest periodicity fₘ and depth m fit
//!   x(t) = A·[1 + m·cos(2π·fₘ·t)]·c(t).
//! - FM: a held note's pitch swings with vibrato, f(t) = f₀ + Δf·cos(2π·fₘ·t),
//!   with modulation index β = Δf/fₘ.
//! - Filters shape the spectrum: its centroid (brightness), the frequency
//!   under which 85% of the power lies (where a low-pass cuts), and its
//!   flatness, the geometric over the arithmetic mean of the power (0 for a
//!   pure tone, 1 for white noise).

use std::collections::VecDeque;
use std::f32::consts::TAU;

use librespot_playback::SAMPLE_RATE;

use super::bin_hz;

/// The envelope: an RMS every 10 ms, the last 5.12 s of them.
pub const ENVELOPE_RATE: f32 = 100.0;
const ENVELOPE_FRAMES: usize = SAMPLE_RATE as usize / 100;
pub const ENVELOPE_POINTS: usize = 512;
/// The envelope is read again after this many new points.
const ENVELOPE_EVERY: usize = 10;
/// The modulation rates looked for in the envelope, from a slow pulse to a
/// fast tremolo, and the shallowest depth reported.
const AM_LOW: f32 = 0.5;
const AM_HIGH: f32 = 20.0;
const AM_STEP: f32 = 0.02;
const AM_SHALLOWEST: f32 = 0.05;
/// The pitch track vibrato is read from: a fundamental 50 times a second.
pub const TRACK_RATE: f32 = 50.0;
/// The held note vibrato is read over, 1.28 s, and its rates.
pub const FM_POINTS: usize = 64;
const FM_LOW: f32 = 3.0;
const FM_HIGH: f32 = 12.0;
const FM_STEP: f32 = 0.05;
/// A held note stays within this many cents of its median, and swings at
/// least this far to count as vibrato: a voice's or a violin's runs from
/// 10 to 100 cents, while the pitch estimate itself wavers by a few.
const HELD_CENTS: f32 = 80.0;
const FM_SHALLOWEST: f32 = 8.0;
/// The spectrum's shape is read from 20 Hz to 20 kHz, its flatness from
/// 50 Hz to 16 kHz, and not at all under -90 dBFS.
const SHAPE_LOW_HZ: f32 = 20.0;
const SHAPE_HIGH_HZ: f32 = 20_000.0;
const FLAT_LOW_HZ: f32 = 50.0;
const FLAT_HIGH_HZ: f32 = 16_000.0;
const SHAPE_QUIETEST: f32 = 1e-9;
/// The share of the power under the rolloff.
pub const ROLLOFF_SHARE: f32 = 0.85;

/// The envelope's modulation: its rate in Hz and its depth m, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Am {
    pub rate: f32,
    pub depth: f32,
}

impl Am {
    /// The rate as beats a minute, when it is as slow as a beat.
    pub fn bpm(&self) -> Option<f32> {
        (0.75..=3.5)
            .contains(&self.rate)
            .then_some(self.rate * 60.0)
    }
}

/// Vibrato: its rate fₘ, the peak deviation Δf in Hz and in cents, and the
/// modulation index β = Δf/fₘ.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fm {
    pub rate: f32,
    pub deviation_hz: f32,
    pub deviation_cents: f32,
    pub index: f32,
}

/// The spectrum's shape: centroid and rolloff in Hz, flatness 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub centroid: f32,
    pub rolloff: f32,
    pub flatness: f32,
}

/// `values` less the straight line that fits them best.
fn detrend(values: &[f32]) -> Vec<f32> {
    let n = values.len() as f32;
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = values.iter().sum::<f32>() / n;
    let (mut covariance, mut variance) = (0.0, 0.0);
    for (i, value) in values.iter().enumerate() {
        let dx = i as f32 - mean_x;
        covariance += dx * (value - mean_y);
        variance += dx * dx;
    }
    let slope = if variance > 0.0 {
        covariance / variance
    } else {
        0.0
    };
    values
        .iter()
        .enumerate()
        .map(|(i, value)| value - mean_y - slope * (i as f32 - mean_x))
        .collect()
}

/// The strongest sinusoid in `values`, sampled `rate` times a second,
/// between `low` and `high` Hz in steps of `step`: its frequency and its
/// amplitude, read through a Hann window once the best line is taken out.
fn strongest(values: &[f32], rate: f32, low: f32, high: f32, step: f32) -> Option<(f32, f32)> {
    let n = values.len();
    if n < 8 {
        return None;
    }
    let windowed: Vec<f32> = detrend(values)
        .iter()
        .enumerate()
        .map(|(i, value)| value * (0.5 - 0.5 * (TAU * i as f32 / (n - 1) as f32).cos()))
        .collect();
    let gain = (n - 1) as f32 / 2.0;
    let magnitude = |hz: f32| {
        // A phasor turned one step a sample, rather than a sine and a
        // cosine for every sample.
        let (step_re, step_im) = ((TAU * hz / rate).cos(), -(TAU * hz / rate).sin());
        let (mut turn_re, mut turn_im) = (1.0f32, 0.0f32);
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for value in &windowed {
            re += value * turn_re;
            im += value * turn_im;
            (turn_re, turn_im) = (
                turn_re * step_re - turn_im * step_im,
                turn_re * step_im + turn_im * step_re,
            );
        }
        (re * re + im * im).sqrt()
    };
    let steps = ((high - low) / step).round() as usize;
    let (hz, peak) = (0..=steps)
        .map(|index| {
            let hz = low + index as f32 * step;
            (hz, magnitude(hz))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    Some((hz, 2.0 * peak / gain))
}

/// The amplitude envelope and the modulation found in it.
pub struct Envelope {
    points: VecDeque<f32>,
    sum: f64,
    frames: usize,
    fresh: usize,
    am: Option<Am>,
}

impl Default for Envelope {
    fn default() -> Self {
        Self {
            points: VecDeque::with_capacity(ENVELOPE_POINTS),
            sum: 0.0,
            frames: 0,
            fresh: 0,
            am: None,
        }
    }
}

impl Envelope {
    /// Follows `frames`, left and right, which come after the last ones.
    pub fn push(&mut self, frames: &[[f32; 2]]) {
        for [left, right] in frames {
            let mono = f64::from((left + right) * 0.5);
            self.sum += mono * mono;
            self.frames += 1;
            if self.frames == ENVELOPE_FRAMES {
                if self.points.len() == ENVELOPE_POINTS {
                    self.points.pop_front();
                }
                self.points
                    .push_back((self.sum / ENVELOPE_FRAMES as f64).sqrt() as f32);
                self.sum = 0.0;
                self.frames = 0;
                self.fresh += 1;
            }
        }
        if self.fresh >= ENVELOPE_EVERY {
            self.fresh = 0;
            self.am = self.measure();
        }
    }

    fn measure(&self) -> Option<Am> {
        if self.points.len() < ENVELOPE_POINTS / 2 {
            return None;
        }
        let values: Vec<f32> = self.points.iter().copied().collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        if mean < 1e-4 {
            return None;
        }
        let (rate, amplitude) = strongest(&values, ENVELOPE_RATE, AM_LOW, AM_HIGH, AM_STEP)?;
        let depth = (amplitude / mean).min(1.0);
        (depth >= AM_SHALLOWEST).then_some(Am { rate, depth })
    }

    /// The envelope's points, oldest first.
    pub fn points(&self) -> &VecDeque<f32> {
        &self.points
    }

    pub fn am(&self) -> Option<Am> {
        self.am
    }
}

/// The vibrato on the note held at the end of `track`, a fundamental or
/// none `TRACK_RATE` times a second, newest last. None unless the last
/// 1.28 s hold one note that swings.
pub fn vibrato(track: &[Option<f32>]) -> Option<Fm> {
    let recent = &track[track.len().checked_sub(FM_POINTS)?..];
    let hz: Vec<f32> = recent.iter().copied().collect::<Option<_>>()?;
    let mut sorted = hz.clone();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[FM_POINTS / 2];
    let cents: Vec<f32> = hz.iter().map(|f| 1200.0 * (f / median).log2()).collect();
    if cents.iter().any(|c| c.abs() > HELD_CENTS) {
        return None;
    }
    let (rate, swing) = strongest(&cents, TRACK_RATE, FM_LOW, FM_HIGH, FM_STEP)?;
    if swing < FM_SHALLOWEST {
        return None;
    }
    let deviation_hz = median * (2f32.powf(swing / 1200.0) - 1.0);
    Some(Fm {
        rate,
        deviation_hz,
        deviation_cents: swing,
        index: deviation_hz / rate,
    })
}

/// The shape of a spectrum in dBFS per bin. None in silence.
pub fn shape(db: &[f32]) -> Option<Shape> {
    let low = (SHAPE_LOW_HZ / bin_hz()).ceil() as usize;
    let high = ((SHAPE_HIGH_HZ / bin_hz()) as usize).min(db.len().checked_sub(1)?);
    let power = |bin: usize| 10f32.powf(db[bin] / 10.0);
    let total: f32 = (low..=high).map(power).sum();
    if total < SHAPE_QUIETEST {
        return None;
    }
    let centroid = (low..=high)
        .map(|bin| bin as f32 * bin_hz() * power(bin))
        .sum::<f32>()
        / total;
    let mut below = 0.0;
    let rolloff = (low..=high)
        .find(|bin| {
            below += power(*bin);
            below >= ROLLOFF_SHARE * total
        })
        .map_or(SHAPE_HIGH_HZ, |bin| bin as f32 * bin_hz());
    // The geometric mean of the power is the power of the mean level.
    let flat_low = (FLAT_LOW_HZ / bin_hz()).ceil() as usize;
    let flat_high = ((FLAT_HIGH_HZ / bin_hz()) as usize).min(high);
    let bins = (flat_high - flat_low + 1) as f32;
    let mean_db = db[flat_low..=flat_high].iter().sum::<f32>() / bins;
    let arithmetic = (flat_low..=flat_high).map(power).sum::<f32>() / bins;
    Some(Shape {
        centroid,
        rolloff,
        flatness: (10f32.powf(mean_db / 10.0) / arithmetic).clamp(0.0, 1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyser::{FFT_SIZE, decibels};

    fn spectrum(wave: &[f32]) -> Vec<f32> {
        let mut magnitudes = vec![0.0; FFT_SIZE / 2];
        crate::vis::Fft::new(FFT_SIZE).magnitudes(wave, &mut magnitudes);
        magnitudes
            .iter()
            .map(|m| decibels(m * 4.0 / FFT_SIZE as f32))
            .collect()
    }

    fn noise(count: usize) -> Vec<f32> {
        let mut seed = 0x2545_f491_u32;
        (0..count)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed as f32 / u32::MAX as f32 - 0.5) * 0.5
            })
            .collect()
    }

    /// A tone whose amplitude swings at 4 Hz by half reads as AM with
    /// fₘ = 4 Hz and m = 0.5.
    #[test]
    fn a_tremolo_reads_as_its_rate_and_depth() {
        let mut envelope = Envelope::default();
        let rate = SAMPLE_RATE as f32;
        let frames: Vec<[f32; 2]> = (0..SAMPLE_RATE as usize * 6)
            .map(|i| {
                let t = i as f32 / rate;
                let sample = 0.4 * (1.0 + 0.5 * (TAU * 4.0 * t).cos()) * (TAU * 1000.0 * t).sin();
                [sample, sample]
            })
            .collect();
        envelope.push(&frames);
        let am = envelope.am().unwrap();
        assert!((am.rate - 4.0).abs() < 0.1, "{} Hz", am.rate);
        assert!((am.depth - 0.5).abs() < 0.1, "m = {}", am.depth);
        assert_eq!(am.bpm(), None, "a tremolo, not a beat");
        assert_eq!(
            Am {
                rate: 2.0,
                depth: 0.3
            }
            .bpm(),
            Some(120.0)
        );
    }

    #[test]
    fn a_steady_tone_has_no_modulation() {
        let mut envelope = Envelope::default();
        let frames: Vec<[f32; 2]> = (0..SAMPLE_RATE as usize * 6)
            .map(|i| {
                let sample = 0.4 * (TAU * 440.0 * i as f32 / SAMPLE_RATE as f32).sin();
                [sample, sample]
            })
            .collect();
        envelope.push(&frames);
        assert_eq!(envelope.am(), None);
    }

    /// A 440 Hz note swinging ±20 cents at 6 Hz: fₘ = 6 Hz, Δf about
    /// 5.1 Hz, β = Δf/fₘ about 0.85.
    #[test]
    fn vibrato_reads_as_its_rate_deviation_and_index() {
        let track: Vec<Option<f32>> = (0..100)
            .map(|i| {
                let cents = 20.0 * (TAU * 6.0 * i as f32 / TRACK_RATE).sin();
                Some(440.0 * 2f32.powf(cents / 1200.0))
            })
            .collect();
        let fm = vibrato(&track).unwrap();
        assert!((fm.rate - 6.0).abs() < 0.2, "{} Hz", fm.rate);
        assert!(
            (fm.deviation_cents - 20.0).abs() < 4.0,
            "{}¢",
            fm.deviation_cents
        );
        assert!(
            (fm.deviation_hz - 5.1).abs() < 1.0,
            "{} Hz",
            fm.deviation_hz
        );
        assert!((fm.index - fm.deviation_hz / fm.rate).abs() < 1e-6);
        let mut changed = track.clone();
        changed[90] = Some(523.0);
        assert_eq!(vibrato(&changed), None, "a new note is not vibrato");
        changed[90] = None;
        assert_eq!(vibrato(&changed), None, "nor is a gap");
    }

    #[test]
    fn a_tone_is_dark_and_peaked_and_noise_is_flat() {
        let wave: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 * (TAU * 1000.0 * i as f32 / SAMPLE_RATE as f32).sin())
            .collect();
        let tone = shape(&spectrum(&wave)).unwrap();
        assert!(
            (tone.centroid - 1000.0).abs() < 50.0,
            "{} Hz",
            tone.centroid
        );
        assert!(tone.rolloff < 1100.0, "{} Hz", tone.rolloff);
        assert!(tone.flatness < 0.05, "{}", tone.flatness);
        let hiss = shape(&spectrum(&noise(FFT_SIZE))).unwrap();
        assert!(hiss.flatness > 0.3, "{}", hiss.flatness);
        assert!(hiss.rolloff > 12_000.0, "{} Hz", hiss.rolloff);
        assert_eq!(shape(&vec![-200.0; FFT_SIZE / 2]), None);
    }
}
