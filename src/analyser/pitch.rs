//! The note sounding: its fundamental, found by the harmonic product
//! spectrum, how clearly the sound is built on it, the level of each of its
//! harmonics, and the energy of each of the twelve pitch classes.
//!
//! A sound with pitch is a sum of sines at whole multiples of one
//! frequency, f_k = k·f₀. Multiplying the spectrum by copies of itself
//! squeezed by 2, 3, 4 and 5 lines those multiples up on f₀, so the product
//! peaks there even when f₀ itself is missing, as the ear hears it. Here
//! the product runs over the spectrum's peaks: each candidate f₀, an eighth
//! of a semitone apart, scores the peaks found where its harmonics fall.

use super::{FLOOR_DB, bin_hz};

/// Harmonics whose levels are kept: the fundamental and seven above it.
pub const HARMONICS: usize = 8;
/// Harmonics multiplied together when looking for the fundamental.
const PRODUCT: usize = 5;
/// Where a fundamental is looked for: a bass guitar's low notes to a
/// soprano's top, a candidate every eighth of a semitone.
const LOWEST_HZ: f32 = 50.0;
const HIGHEST_HZ: f32 = 1_600.0;
const CANDIDATES_PER_OCTAVE: f32 = 96.0;
/// A harmonic is found within this share of where it should fall, half a
/// semitone, and never closer than this many bins: a parabola places an
/// isolated partial to within a twentieth of one.
const HARMONIC_SHARE: f32 = 0.029;
const HARMONIC_LEAST_BINS: f32 = 0.3;
/// The quietest harmonic that names a fundamental, and the quietest peak
/// that counts at all.
const QUIETEST_DB: f32 = -60.0;
const PEAK_FLOOR_DB: f32 = FLOOR_DB + 6.0;
/// A harmonic counts by how far it stands above a line this far under the
/// spectrum's loudest peak. The window's sidelobes sit 31 dB and more
/// under their partial, so they count for next to nothing.
const HARMONIC_RANGE_DB: f32 = 40.0;
/// Harmonics counted when measuring clarity.
const CLARITY_HARMONICS: usize = 16;
/// Clarity is measured up to here, and pitch classes read between these:
/// below 150 Hz a semitone is narrower than a bin.
const CLARITY_HIGH_HZ: f32 = 5_000.0;
const CHROMA_LOW_HZ: f32 = 150.0;
const CHROMA_HIGH_HZ: f32 = 5_000.0;
/// Below this total power, -60 dBFS, there are no pitch classes to show.
const CHROMA_QUIETEST: f32 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pitch {
    /// The fundamental, in Hz.
    pub hz: f32,
    /// How much more of the power sits on its harmonics than chance would
    /// put there, from 0 (none) to 1 (all of it).
    pub clarity: f32,
}

/// The spectrum's peaks up to `last`, each placed between bins by a
/// parabola through it and its neighbours, as (bin, level), in order. The
/// skirts the window spreads around a partial are not peaks, so a
/// neighbouring note's skirt is never taken for a harmonic.
fn peaks(db: &[f32], last: usize) -> Vec<(f32, f32)> {
    (2..last.min(db.len().saturating_sub(2)))
        .filter(|bin| db[*bin] > PEAK_FLOOR_DB && db[*bin] > db[bin - 1] && db[*bin] >= db[bin + 1])
        .map(|bin| {
            let (before, at, after) = (db[bin - 1], db[bin], db[bin + 1]);
            let curve = before - 2.0 * at + after;
            let offset = if curve.abs() > f32::EPSILON {
                (0.5 * (before - after) / curve).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            (bin as f32 + offset, at)
        })
        .collect()
}

/// The peak nearest `bin` within `reach` bins of it.
fn peak_at(peaks: &[(f32, f32)], bin: f32, reach: f32) -> Option<(f32, f32)> {
    let after = peaks.partition_point(|(at, _)| *at < bin);
    [after.checked_sub(1), Some(after)]
        .into_iter()
        .flatten()
        .filter_map(|index| peaks.get(index).copied())
        .filter(|(at, _)| (at - bin).abs() <= reach)
        .min_by(|a, b| (a.0 - bin).abs().total_cmp(&(b.0 - bin).abs()))
}

/// How far from `bin` a harmonic may fall and still be it.
fn reach(bin: f32) -> f32 {
    (bin * HARMONIC_SHARE).max(HARMONIC_LEAST_BINS)
}

/// The fundamental of a spectrum in dBFS per bin, and how clearly the sound
/// is built on it. None in silence, or when no harmonic is loud enough.
pub fn fundamental(db: &[f32]) -> Option<Pitch> {
    let top = HIGHEST_HZ / bin_hz() * (PRODUCT as f32 + 0.5);
    let peaks = peaks(db, top.ceil() as usize + 2);
    let loudest = peaks
        .iter()
        .map(|(_, level)| *level)
        .fold(FLOOR_DB, f32::max);
    if loudest < QUIETEST_DB {
        return None;
    }
    let line = loudest - HARMONIC_RANGE_DB;
    // The product of the squeezed spectra, as a sum of decibels above the
    // line: a harmonic with no peak where it falls adds nothing.
    let score = |bin: f32| {
        (1..=PRODUCT)
            .map(|h| {
                let at = bin * h as f32;
                peak_at(&peaks, at, reach(at)).map_or(0.0, |(_, level)| (level - line).max(0.0))
            })
            .sum::<f32>()
    };
    let octaves = (HIGHEST_HZ / LOWEST_HZ).log2();
    let count = (octaves * CANDIDATES_PER_OCTAVE) as usize;
    // Candidates on a plateau of one score share their peaks; the highest
    // of them is taken, so a lone partial is its own fundamental rather
    // than the fifth harmonic of one far below.
    let best = (0..=count)
        .map(|step| LOWEST_HZ * 2f32.powf(step as f32 / CANDIDATES_PER_OCTAVE) / bin_hz())
        .map(|bin| (bin, score(bin)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?
        .0;
    // Each harmonic found names the fundamental again, the higher ones more
    // finely; weighed by their amplitudes they agree on a value far finer
    // than one bin.
    let (mut sum, mut weight, mut strongest) = (0.0, 0.0, FLOOR_DB);
    for h in 1..=PRODUCT {
        let at = best * h as f32;
        if let Some((found, level)) = peak_at(&peaks, at, reach(at)).filter(|(_, l)| *l > line) {
            let amplitude = 10f32.powf(level / 20.0);
            sum += found / h as f32 * amplitude;
            weight += amplitude;
            strongest = strongest.max(level);
        }
    }
    if weight <= 0.0 || strongest < QUIETEST_DB {
        return None;
    }
    let hz = sum / weight * bin_hz();
    Some(Pitch {
        hz,
        clarity: clarity(db, hz),
    })
}

/// The share of the power up to 5 kHz that sits within a bin of the first
/// harmonics of `hz`, above the share those bins would hold of noise.
fn clarity(db: &[f32], hz: f32) -> f32 {
    let low = (LOWEST_HZ / bin_hz()) as usize;
    let high = ((CLARITY_HIGH_HZ / bin_hz()) as usize).min(db.len() - 1);
    let mut on = vec![false; high + 1];
    for h in 1..=CLARITY_HARMONICS {
        let centre = (hz * h as f32 / bin_hz()).round() as usize;
        if centre > high {
            break;
        }
        for slot in &mut on[centre.saturating_sub(1)..=(centre + 1).min(high)] {
            *slot = true;
        }
    }
    let (mut total, mut harmonic, mut covered) = (0.0f32, 0.0f32, 0usize);
    for (bin, level) in db.iter().enumerate().take(high + 1).skip(low) {
        let power = 10f32.powf(level / 10.0);
        total += power;
        if on[bin] {
            harmonic += power;
            covered += 1;
        }
    }
    let chance = covered as f32 / (high + 1 - low) as f32;
    if total <= 0.0 || chance >= 1.0 {
        return 0.0;
    }
    ((harmonic / total - chance) / (1.0 - chance)).clamp(0.0, 1.0)
}

/// The level of each of the first harmonics of `hz`, the loudest bin beside
/// where each falls, in dBFS; the floor past the top of the spectrum.
pub fn harmonics(db: &[f32], hz: f32) -> [f32; HARMONICS] {
    std::array::from_fn(|k| {
        let bin = (hz * (k + 1) as f32 / bin_hz()).round() as usize;
        if bin == 0 || bin + 1 >= db.len() {
            FLOOR_DB
        } else {
            db[bin - 1..=bin + 1]
                .iter()
                .copied()
                .fold(FLOOR_DB, f32::max)
        }
    })
}

/// The power of each pitch class, C first, from 150 Hz to 5 kHz, with the
/// loudest at 1; all naught in silence.
pub fn chroma(db: &[f32]) -> [f32; 12] {
    let mut classes = [0.0f32; 12];
    let low = (CHROMA_LOW_HZ / bin_hz()).ceil() as usize;
    let high = ((CHROMA_HIGH_HZ / bin_hz()) as usize).min(db.len().saturating_sub(1));
    for (bin, level) in db.iter().enumerate().take(high + 1).skip(low) {
        let hz = bin as f32 * bin_hz();
        let class = ((69.0 + 12.0 * (hz / 440.0).log2()).round() as i32).rem_euclid(12) as usize;
        classes[class] += 10f32.powf(level / 10.0);
    }
    let total: f32 = classes.iter().sum();
    let loudest = classes.iter().copied().fold(0.0, f32::max);
    if total < CHROMA_QUIETEST || loudest <= 0.0 {
        return [0.0; 12];
    }
    classes.map(|power| power / loudest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyser::{FFT_SIZE, decibels};
    use librespot_playback::SAMPLE_RATE;

    /// The spectrum the analyser measures, in dBFS per bin, of partials
    /// given as (frequency, amplitude).
    fn spectrum(partials: &[(f32, f32)]) -> Vec<f32> {
        let wave: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                partials
                    .iter()
                    .map(|(hz, amplitude)| amplitude * (std::f32::consts::TAU * hz * t).sin())
                    .sum()
            })
            .collect();
        let mut magnitudes = vec![0.0; FFT_SIZE / 2];
        crate::vis::Fft::new(FFT_SIZE).magnitudes(&wave, &mut magnitudes);
        magnitudes
            .iter()
            .map(|m| decibels(m * 4.0 / FFT_SIZE as f32))
            .collect()
    }

    fn tone(hz: f32, from: usize, to: usize) -> Vec<(f32, f32)> {
        (from..=to)
            .map(|k| (hz * k as f32, 0.4 / k as f32))
            .collect()
    }

    #[test]
    fn a_harmonic_tone_names_its_fundamental_finely() {
        let pitch = fundamental(&spectrum(&tone(220.0, 1, 6))).unwrap();
        assert!((pitch.hz - 220.0).abs() < 1.0, "{} Hz", pitch.hz);
        assert!(pitch.clarity > 0.6, "clarity {}", pitch.clarity);
        let low = fundamental(&spectrum(&tone(82.4, 1, 8))).unwrap();
        assert!((low.hz - 82.4).abs() < 1.0, "a bass E: {} Hz", low.hz);
    }

    /// The ear hears a missing fundamental from its harmonics alone; so
    /// does the product.
    #[test]
    fn a_missing_fundamental_is_still_found() {
        let pitch = fundamental(&spectrum(&tone(200.0, 2, 6))).unwrap();
        assert!((pitch.hz - 200.0).abs() < 1.5, "{} Hz", pitch.hz);
    }

    /// Another note just under where a harmonic would be does not drag the
    /// fundamental down with it.
    #[test]
    fn a_neighbouring_note_does_not_pull_the_fundamental() {
        let pitch = fundamental(&spectrum(&[(55.0, 0.16), (110.0, 0.07), (146.8, 0.15)])).unwrap();
        assert!((pitch.hz - 55.0).abs() < 1.0, "{} Hz", pitch.hz);
    }

    #[test]
    fn silence_has_no_note_and_no_pitch_classes() {
        let silent = vec![FLOOR_DB; FFT_SIZE / 2];
        assert_eq!(fundamental(&silent), None);
        assert_eq!(chroma(&silent), [0.0; 12]);
    }

    #[test]
    fn harmonics_fall_where_the_tone_puts_them() {
        let db = spectrum(&tone(220.0, 1, 4));
        let levels = harmonics(&db, 220.0);
        assert!(levels[0] > levels[1] && levels[1] > levels[2], "{levels:?}");
        assert!(levels[5] < -60.0, "no sixth harmonic: {}", levels[5]);
    }

    #[test]
    fn an_a_lights_the_a_pitch_class() {
        let classes = chroma(&spectrum(&[(440.0, 0.5), (880.0, 0.3)]));
        assert_eq!(classes[9], 1.0);
        assert!(
            classes
                .iter()
                .enumerate()
                .all(|(class, power)| class == 9 || *power < 0.2),
            "{classes:?}"
        );
    }
}
