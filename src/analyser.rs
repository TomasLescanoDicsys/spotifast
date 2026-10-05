//! The pro analyser's measurements, for a large view of the sound with its
//! scales: a spectrum in dBFS with peak hold, a scope that triggers on a
//! rising edge and fits its gain to the wave, sample peak and RMS levels, a
//! spectrogram of the last ten seconds, the note sounding with its
//! harmonics ([`pitch`]), loudness ([`loudness`]), and the sound's
//! modulations and filtering ([`modulation`]).
//!
//! It reads the tap every visualiser reads, post-equalizer and pre-volume,
//! through a transform eight times longer than the classic analyser's, so
//! bins sit 10.8 Hz apart instead of 86 Hz.

pub mod loudness;
pub mod modulation;
pub mod pitch;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use librespot_playback::SAMPLE_RATE;

use crate::vis::{AudioTap, Fft, LAG};

/// Samples in one transform: 93 ms of sound.
pub const FFT_SIZE: usize = 4096;
const BINS: usize = FFT_SIZE / 2;
/// The quietest level shown, in dBFS.
pub const FLOOR_DB: f32 = -96.0;
/// The ends of the frequency axis, which is logarithmic.
pub const LOW_HZ: f32 = 20.0;
pub const HIGH_HZ: f32 = 20_000.0;
/// How fast the shown spectrum falls back, in dB a second. It rises at once.
const RELEASE: f32 = 48.0;
/// How long a peak holds, in seconds, then how fast it falls, in dB a second.
const PEAK_HOLD: f32 = 1.5;
const PEAK_FALL: f32 = 12.0;
/// Sample peak and RMS are measured over 300 ms, as a VU meter integrates.
const LEVEL_SAMPLES: usize = SAMPLE_RATE as usize * 3 / 10;
/// The spectrogram: a column every `HOP`, `HISTORY` of them (ten seconds),
/// each `ROWS` tall along the logarithmic frequency axis.
pub const HOP: Duration = Duration::from_millis(20);
const HOP_SAMPLES: usize = SAMPLE_RATE as usize / 50;
pub const HISTORY: usize = 500;
pub const ROWS: usize = 256;
/// The most columns one frame writes after a stall: as far back as the
/// tap still holds a whole transform's worth of sound.
const CATCH_UP: usize = 12;
/// The scope's divisions across, the time each may span in milliseconds,
/// and its vertical gains, the first of them fitted to the wave.
pub const DIVISIONS: usize = 10;
pub const TIME_BASES: [f32; 7] = [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0];
pub const GAINS: [f32; 6] = [0.0, 0.5, 1.0, 2.0, 4.0, 8.0];
/// The fitted gain puts the wave's peak this far up the half-height, drops
/// at once for a louder wave, rises over about this long for a quieter one,
/// and stays within these bounds.
const AUTO_FILL: f32 = 0.85;
const AUTO_RELEASE: f32 = 0.8;
const AUTO_LEAST: f32 = 0.25;
const AUTO_MOST: f32 = 64.0;
/// Earlier traces the scope keeps for its phosphor's afterglow.
pub const TRAILS: usize = 8;
/// The sound the scope keeps: twice its widest span, the older half to
/// find a trigger in.
const SCOPE_KEPT: usize = SAMPLE_RATE as usize / 5;
/// Below this the scope free-runs rather than chase a trigger in noise.
const SILENT: f32 = 1e-3;
/// Samples averaged before looking for a trigger, so hiss does not trip it.
const SMOOTHING: usize = 8;
/// A fundamental this clear names the note sounding.
pub const NAMED_CLARITY: f32 = 0.2;
/// The rolloff's sweep is read between columns this far apart, one second.
const SWEEP_COLUMNS: usize = 50;

/// Hertz between one bin and the next.
pub fn bin_hz() -> f32 {
    SAMPLE_RATE as f32 / FFT_SIZE as f32
}

/// `amplitude`, where 1 is full scale, in dBFS, never below the floor.
pub fn decibels(amplitude: f32) -> f32 {
    (20.0 * amplitude.max(1e-9).log10()).max(FLOOR_DB)
}

/// The frequency at `t`, from 0 to 1, along the logarithmic axis.
pub fn hz_at(t: f32) -> f32 {
    LOW_HZ * (HIGH_HZ / LOW_HZ).powf(t)
}

/// Where `hz` falls along the logarithmic axis, 0 at the low end and 1 at
/// the high.
pub fn position_of(hz: f32) -> f32 {
    (hz / LOW_HZ).ln() / (HIGH_HZ / LOW_HZ).ln()
}

/// The level of the band from `low` to `high` Hz in a spectrum of dBFS per
/// bin: the loudest bin inside it, or, where the band is narrower than a
/// bin, the level read between the two nearest.
pub fn band_level(db: &[f32], low: f32, high: f32) -> f32 {
    let Some(last) = db.len().checked_sub(1) else {
        return FLOOR_DB;
    };
    let (from, to) = (low / bin_hz(), high / bin_hz());
    if to - from < 1.0 {
        let centre = ((from + to) / 2.0).clamp(0.0, last as f32);
        let below = centre.floor() as usize;
        let above = (below + 1).min(last);
        let t = centre - below as f32;
        return db[below] + (db[above] - db[below]) * t;
    }
    let start = (from.ceil() as usize).min(last);
    let end = (to.floor() as usize).min(last);
    db[start..=end].iter().copied().fold(FLOOR_DB, f32::max)
}

/// The note nearest `hz` in equal temperament with A4 at 440 Hz: its pitch
/// class (C is 0), its octave, and how many cents `hz` sits above it (below
/// when negative). The MIDI number is n = 69 + 12·log₂(f/440).
pub fn note_parts(hz: f32) -> Option<(usize, i32, i32)> {
    if !hz.is_finite() || hz <= 0.0 {
        return None;
    }
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let nearest = midi.round();
    let cents = ((midi - nearest) * 100.0).round() as i32;
    let number = nearest as i32;
    Some((
        number.rem_euclid(12) as usize,
        number.div_euclid(12) - 1,
        cents,
    ))
}

/// The note nearest `hz` by its letter name and octave, and its cents.
pub fn note(hz: f32) -> Option<(String, i32)> {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let (class, octave, cents) = note_parts(hz)?;
    Some((format!("{}{octave}", NAMES[class]), cents))
}

/// Where the latest rising zero crossing is that still leaves `count`
/// samples after it, so a steady tone stands still on the scope. The
/// search reads a lightly smoothed copy, centred so the crossing stays
/// where it is, and gives up on silence.
pub fn trigger(samples: &[f32], count: usize) -> Option<usize> {
    let last = samples.len().checked_sub(count)?;
    if samples.iter().all(|sample| sample.abs() < SILENT) {
        return None;
    }
    // Eight samples centred half a sample after `i`, so a crossing found
    // between `i - 1` and `i` lands on the sample nearest it.
    let smooth = |i: usize| {
        let from = i.saturating_sub(SMOOTHING / 2 - 1);
        let run = &samples[from..(i + SMOOTHING / 2 + 1).min(samples.len())];
        run.iter().sum::<f32>() / run.len() as f32
    };
    (1..=last)
        .rev()
        .find(|&i| smooth(i - 1) < 0.0 && smooth(i) >= 0.0)
}

/// Sample peak and RMS over the last 300 ms, in dBFS, and the highest peak
/// of the last moments, held and then falling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    pub peak: f32,
    pub rms: f32,
    pub held: f32,
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            peak: FLOOR_DB,
            rms: FLOOR_DB,
            held: FLOOR_DB,
        }
    }
}

/// The pro analyser's memory between frames.
pub struct ProAnalyser {
    fft: Fft,
    magnitudes: Vec<f32>,
    /// The spectrum as measured this frame, and as shown, in dBFS per bin.
    measured: Vec<f32>,
    shown: Vec<f32>,
    /// Each bin's peak, and how long it has held there.
    peaks: Vec<f32>,
    held: Vec<f32>,
    levels: Levels,
    level_held: f32,
    /// This frame's fundamental, its harmonics' levels, the pitch classes'
    /// energies, and the spectrum's shape.
    pitch: Option<pitch::Pitch>,
    harmonics: [f32; pitch::HARMONICS],
    chroma: [f32; 12],
    shape: Option<modulation::Shape>,
    /// Loudness and the envelope read every frame of the stereo sound once;
    /// `read` is where they reached.
    loudness: loudness::Meter,
    envelope: modulation::Envelope,
    read: u64,
    /// The song measured, so a new one starts its loudness afresh.
    song: Option<String>,
    /// The latest stretch of sound for the scope, kept so a paused song can
    /// still be read at any time base.
    scope: Vec<f32>,
    /// The scope's earlier traces, oldest first, and its fitted gain.
    trails: VecDeque<Vec<f32>>,
    auto_gain: f32,
    /// The spectrogram: a ring of `HISTORY` columns of `ROWS` levels, bass
    /// first, each from 0 (the floor) to 255 (full scale), with each
    /// column's named fundamental and rolloff. `next` is the column written
    /// next, which is also the oldest.
    columns: Vec<u8>,
    notes: Vec<Option<f32>>,
    rolloffs: Vec<f32>,
    next: usize,
    /// Columns written since the view last took them.
    fresh: Vec<usize>,
    last: Option<Instant>,
    last_column: Option<Instant>,
    /// Which of `TIME_BASES` and `GAINS` the scope draws at.
    time_base: usize,
    gain: usize,
}

impl Default for ProAnalyser {
    fn default() -> Self {
        Self {
            fft: Fft::new(FFT_SIZE),
            magnitudes: vec![0.0; BINS],
            measured: vec![FLOOR_DB; BINS],
            shown: vec![FLOOR_DB; BINS],
            peaks: vec![FLOOR_DB; BINS],
            held: vec![0.0; BINS],
            levels: Levels::default(),
            level_held: 0.0,
            pitch: None,
            harmonics: [FLOOR_DB; pitch::HARMONICS],
            chroma: [0.0; 12],
            shape: None,
            loudness: loudness::Meter::default(),
            envelope: modulation::Envelope::default(),
            read: 0,
            song: None,
            scope: vec![0.0; SCOPE_KEPT],
            trails: VecDeque::with_capacity(TRAILS),
            auto_gain: 1.0,
            columns: vec![0; HISTORY * ROWS],
            notes: vec![None; HISTORY],
            rolloffs: vec![f32::NAN; HISTORY],
            next: 0,
            fresh: Vec::new(),
            last: None,
            last_column: None,
            time_base: 3,
            gain: 0,
        }
    }
}

impl ProAnalyser {
    /// One frame of sound from the tap: the spectrum, levels, and scope
    /// move to it, loudness and the envelope take in every frame since the
    /// last, and the spectrogram gains a column for each `HOP` gone by.
    pub fn step(&mut self, tap: &AudioTap, now: Instant) {
        let elapsed = self.last.map_or(0.0, |last| {
            now.saturating_duration_since(last)
                .min(Duration::from_millis(250))
                .as_secs_f32()
        });
        self.last = Some(now);
        self.measure(&tap.window(FFT_SIZE, LAG), elapsed);
        self.measure_levels(&tap.window(LEVEL_SAMPLES, LAG), elapsed);
        let (frames, read) = tap.stereo_since(self.read, LAG);
        self.read = read;
        self.loudness.push(&frames);
        self.envelope.push(&frames);
        self.scope = tap.window(SCOPE_KEPT, LAG);
        self.follow_wave(elapsed);
        let due = match self.last_column {
            None => {
                self.last_column = Some(now);
                1
            }
            Some(at) => {
                let due =
                    (now.saturating_duration_since(at).as_micros() / HOP.as_micros()) as usize;
                if due > CATCH_UP {
                    self.last_column = Some(now);
                    CATCH_UP
                } else {
                    self.last_column = Some(at + HOP * due as u32);
                    due
                }
            }
        };
        for back in (0..due).rev() {
            self.write_column(&tap.window(FFT_SIZE, LAG + back * HOP_SAMPLES));
        }
    }

    /// The song stopped sounding here: everything holds where it is, and
    /// the next step starts the clock again rather than catch up.
    pub fn pause(&mut self) {
        self.last = None;
        self.last_column = None;
    }

    /// Notes the song playing; a different one starts loudness, the
    /// envelope and the afterglow afresh.
    pub fn follow(&mut self, song: &str) {
        if self.song.as_deref() != Some(song) {
            self.song = Some(song.to_owned());
            self.loudness = loudness::Meter::default();
            self.envelope = modulation::Envelope::default();
            self.trails.clear();
        }
    }

    fn measure(&mut self, samples: &[f32], elapsed: f32) {
        self.fft.magnitudes(samples, &mut self.magnitudes);
        // A full-scale sine reads 0 dBFS: the Hann window halves the bin,
        // and one side of the spectrum holds half of the sine.
        let scale = 4.0 / FFT_SIZE as f32;
        for (bin, magnitude) in self.magnitudes.iter().enumerate() {
            let level = decibels(magnitude * scale);
            self.measured[bin] = level;
            self.shown[bin] = (self.shown[bin] - RELEASE * elapsed).max(level);
            if level >= self.peaks[bin] {
                self.peaks[bin] = level;
                self.held[bin] = 0.0;
            } else if self.held[bin] < PEAK_HOLD {
                self.held[bin] += elapsed;
            } else {
                self.peaks[bin] = (self.peaks[bin] - PEAK_FALL * elapsed).max(self.shown[bin]);
            }
        }
        self.pitch = pitch::fundamental(&self.measured);
        self.harmonics = self.pitch.map_or([FLOOR_DB; pitch::HARMONICS], |found| {
            pitch::harmonics(&self.measured, found.hz)
        });
        self.chroma = pitch::chroma(&self.measured);
        self.shape = modulation::shape(&self.measured);
    }

    fn measure_levels(&mut self, samples: &[f32], elapsed: f32) {
        let peak = samples
            .iter()
            .fold(0.0f32, |loudest, sample| loudest.max(sample.abs()));
        let power =
            samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len().max(1) as f32;
        self.levels.peak = decibels(peak);
        self.levels.rms = decibels(power.sqrt());
        if self.levels.peak >= self.levels.held {
            self.levels.held = self.levels.peak;
            self.level_held = 0.0;
        } else if self.level_held < PEAK_HOLD {
            self.level_held += elapsed;
        } else {
            self.levels.held = (self.levels.held - PEAK_FALL * elapsed).max(self.levels.peak);
        }
    }

    /// Keeps this frame's trace for the afterglow, and fits the gain to it:
    /// down at once for a louder wave, so it never clips, and back up
    /// slowly for a quieter one, so it does not pump.
    fn follow_wave(&mut self, elapsed: f32) {
        let trace = self.trace().0.to_vec();
        let peak = trace
            .iter()
            .fold(0.0f32, |loudest, sample| loudest.max(sample.abs()));
        let target = (AUTO_FILL / peak.max(AUTO_FILL / AUTO_MOST)).clamp(AUTO_LEAST, AUTO_MOST);
        self.auto_gain = if target < self.auto_gain {
            target
        } else {
            self.auto_gain + (target - self.auto_gain) * (1.0 - (-elapsed / AUTO_RELEASE).exp())
        };
        if self.trails.len() == TRAILS {
            self.trails.pop_front();
        }
        self.trails.push_back(trace);
    }

    fn write_column(&mut self, samples: &[f32]) {
        self.fft.magnitudes(samples, &mut self.magnitudes);
        let scale = 4.0 / FFT_SIZE as f32;
        for magnitude in &mut self.magnitudes {
            *magnitude = decibels(*magnitude * scale);
        }
        let column = &mut self.columns[self.next * ROWS..(self.next + 1) * ROWS];
        for (row, slot) in column.iter_mut().enumerate() {
            let low = hz_at(row as f32 / ROWS as f32);
            let high = hz_at((row + 1) as f32 / ROWS as f32);
            let level = band_level(&self.magnitudes, low, high);
            *slot = ((level - FLOOR_DB) / -FLOOR_DB * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
        self.notes[self.next] = pitch::fundamental(&self.magnitudes)
            .filter(|found| found.clarity >= NAMED_CLARITY)
            .map(|found| found.hz);
        self.rolloffs[self.next] =
            modulation::shape(&self.magnitudes).map_or(f32::NAN, |shape| shape.rolloff);
        self.fresh.push(self.next);
        self.next = (self.next + 1) % HISTORY;
    }

    /// The spectrum as shown, in dBFS per bin.
    pub fn spectrum(&self) -> &[f32] {
        &self.shown
    }

    /// Each bin's held peak, in dBFS.
    pub fn peaks(&self) -> &[f32] {
        &self.peaks
    }

    pub fn levels(&self) -> Levels {
        self.levels
    }

    /// The fundamental sounding now, however clear.
    pub fn pitch(&self) -> Option<pitch::Pitch> {
        self.pitch
    }

    /// The levels of the fundamental's first harmonics, in dBFS.
    pub fn harmonics(&self) -> [f32; pitch::HARMONICS] {
        self.harmonics
    }

    /// Each pitch class's energy, C first, the loudest at 1.
    pub fn chroma(&self) -> [f32; 12] {
        self.chroma
    }

    pub fn shape(&self) -> Option<modulation::Shape> {
        self.shape
    }

    pub fn loudness(&self) -> &loudness::Meter {
        &self.loudness
    }

    pub fn envelope(&self) -> &modulation::Envelope {
        &self.envelope
    }

    /// The `count` latest columns' values of `ring`, oldest first.
    fn recent<T: Copy>(&self, ring: &[T], count: usize) -> Vec<T> {
        (HISTORY - count..HISTORY)
            .map(|back| ring[(self.next + back) % HISTORY])
            .collect()
    }

    /// The vibrato on the note held over the last 1.28 s.
    pub fn vibrato(&self) -> Option<modulation::Fm> {
        modulation::vibrato(&self.recent(&self.notes, modulation::FM_POINTS))
    }

    /// The last 1.28 s of named fundamentals, oldest first.
    pub fn note_track(&self) -> Vec<Option<f32>> {
        self.recent(&self.notes, modulation::FM_POINTS)
    }

    /// How fast the rolloff moves, in octaves a second: up as a filter
    /// opens, down as it closes. Each end is the median of five columns.
    pub fn sweep(&self) -> Option<f32> {
        let rolloffs = self.recent(&self.rolloffs, SWEEP_COLUMNS + 5);
        let median = |values: &[f32]| {
            let mut sorted: Vec<f32> = values.iter().copied().filter(|v| v.is_finite()).collect();
            sorted.sort_by(f32::total_cmp);
            (sorted.len() >= 3).then(|| sorted[sorted.len() / 2])
        };
        let before = median(&rolloffs[..5])?;
        let now = median(&rolloffs[SWEEP_COLUMNS..])?;
        Some((now / before).log2() * (1000.0 / (SWEEP_COLUMNS as f32 * HOP.as_millis() as f32)))
    }

    /// The fundamental named in one spectrogram column.
    pub fn column_note(&self, index: usize) -> Option<f32> {
        self.notes[index]
    }

    pub fn time_base(&self) -> usize {
        self.time_base
    }

    /// Changes the time base; the afterglow of the old one goes with it.
    pub fn set_time_base(&mut self, index: usize) {
        let index = index.min(TIME_BASES.len() - 1);
        if index != self.time_base {
            self.time_base = index;
            self.trails.clear();
        }
    }

    /// Which of `GAINS` the scope uses; the first is fitted to the wave.
    pub fn gain_index(&self) -> usize {
        self.gain
    }

    pub fn set_gain_index(&mut self, index: usize) {
        self.gain = index.min(GAINS.len() - 1);
    }

    /// The gain the scope draws at, fitted or chosen.
    pub fn gain(&self) -> f32 {
        if self.gain == 0 {
            self.auto_gain
        } else {
            GAINS[self.gain]
        }
    }

    /// The scope's earlier traces, oldest first.
    pub fn trails(&self) -> &VecDeque<Vec<f32>> {
        &self.trails
    }

    /// Samples the scope spans at its time base.
    pub fn scope_span(&self) -> usize {
        let ms = TIME_BASES[self.time_base] * DIVISIONS as f32;
        ((ms * SAMPLE_RATE as f32 / 1000.0).round() as usize).clamp(2, SCOPE_KEPT / 2)
    }

    /// The stretch of sound the scope draws, and whether it starts at a
    /// trigger rather than free-running.
    pub fn trace(&self) -> (&[f32], bool) {
        let count = self.scope_span().min(self.scope.len());
        match trigger(&self.scope, count) {
            Some(start) => (&self.scope[start..start + count], true),
            None => (&self.scope[self.scope.len() - count..], false),
        }
    }

    /// One spectrogram column, bass first.
    pub fn column(&self, index: usize) -> &[u8] {
        &self.columns[index * ROWS..(index + 1) * ROWS]
    }

    /// The column written next, which is also the oldest.
    pub fn next_column(&self) -> usize {
        self.next
    }

    /// The columns written since this was last asked.
    pub fn take_fresh(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, amplitude: f32, count: usize) -> Vec<f32> {
        (0..count)
            .map(|i| amplitude * (i as f32 / SAMPLE_RATE as f32 * hz * std::f32::consts::TAU).sin())
            .collect()
    }

    /// A tap holding `samples` of mono sound, ahead of the lag the
    /// analyser looks behind.
    fn tap_with(samples: &[f32]) -> std::sync::Arc<AudioTap> {
        let tap = AudioTap::new();
        let mut interleaved: Vec<f64> = samples
            .iter()
            .flat_map(|sample| [f64::from(*sample); 2])
            .collect();
        interleaved.extend(std::iter::repeat_n(0.0, LAG * 2));
        tap.push(&interleaved, 1.0);
        tap
    }

    #[test]
    fn a_full_scale_sine_reads_zero_dbfs_at_its_frequency() {
        let tap = tap_with(&sine(1000.0, 1.0, FFT_SIZE));
        let mut analyser = ProAnalyser::default();
        analyser.step(&tap, Instant::now());
        let level = band_level(analyser.spectrum(), 990.0, 1010.0);
        assert!(level.abs() < 1.5, "{level} dBFS");
        let far = band_level(analyser.spectrum(), 5000.0, 6000.0);
        assert!(far < -60.0, "far from the tone: {far} dBFS");
        let pitch = analyser.pitch().unwrap();
        assert!((pitch.hz - 1000.0).abs() < 2.0, "{} Hz", pitch.hz);
        let half = tap_with(&sine(1000.0, 0.5, FFT_SIZE));
        let mut quieter = ProAnalyser::default();
        quieter.step(&half, Instant::now());
        let level = band_level(quieter.spectrum(), 990.0, 1010.0);
        assert!((level + 6.0).abs() < 1.5, "half scale: {level} dBFS");
    }

    #[test]
    fn levels_read_a_sines_peak_and_rms() {
        let tap = tap_with(&sine(440.0, 0.5, LEVEL_SAMPLES));
        let mut analyser = ProAnalyser::default();
        analyser.step(&tap, Instant::now());
        let levels = analyser.levels();
        assert!((levels.peak + 6.02).abs() < 0.1, "{}", levels.peak);
        assert!((levels.rms + 9.03).abs() < 0.1, "{}", levels.rms);
        assert_eq!(levels.held, levels.peak);
    }

    #[test]
    fn notes_are_named_with_their_cents() {
        assert_eq!(note(440.0), Some(("A4".into(), 0)));
        assert_eq!(note(261.63), Some(("C4".into(), 0)));
        assert_eq!(note(27.5), Some(("A0".into(), 0)));
        let (name, cents) = note(445.0).unwrap();
        assert_eq!(name, "A4");
        assert_eq!(cents, 20);
        assert_eq!(note(460.0), Some(("A#4".into(), -23)));
        assert_eq!(note_parts(460.0), Some((10, 4, -23)));
        assert_eq!(note(0.0), None);
    }

    #[test]
    fn the_axis_runs_from_twenty_hertz_to_twenty_kilohertz() {
        assert!((hz_at(0.0) - LOW_HZ).abs() < 1e-3);
        assert!((hz_at(1.0) - HIGH_HZ).abs() < 0.5);
        assert!((position_of(632.5) - 0.5).abs() < 1e-3);
        assert!((position_of(hz_at(0.3)) - 0.3).abs() < 1e-5);
    }

    #[test]
    fn a_steady_tone_triggers_on_its_rising_edge() {
        let wave = sine(441.0, 0.5, 4000);
        let start = trigger(&wave, 1000).unwrap();
        assert!(start <= 3000);
        // The latest crossing: a period (100 samples) from the end of the
        // search, at the wave's zero within a sample.
        assert!(start > 2900, "{start}");
        assert!(wave[start].abs() < 0.05, "{}", wave[start]);
        assert!(wave[start + 5] > wave[start], "rising");
        assert_eq!(trigger(&[0.0; 4000], 1000), None, "silence free-runs");
        assert_eq!(trigger(&wave, 5000), None, "too short to fill the span");
    }

    #[test]
    fn the_spectrogram_writes_a_column_each_hop() {
        let tap = tap_with(&sine(1000.0, 0.5, FFT_SIZE * 2));
        let mut analyser = ProAnalyser::default();
        let start = Instant::now();
        analyser.step(&tap, start);
        assert_eq!(analyser.take_fresh(), vec![0]);
        analyser.step(&tap, start + HOP / 2);
        assert!(
            analyser.take_fresh().is_empty(),
            "half a hop writes nothing"
        );
        analyser.step(&tap, start + HOP * 3);
        assert_eq!(analyser.take_fresh(), vec![1, 2, 3]);
        assert_eq!(analyser.next_column(), 4);
        let row = (position_of(1000.0) * ROWS as f32) as usize;
        let column = analyser.column(3);
        assert!(column[row] > 200, "the tone is bright: {}", column[row]);
        assert!(
            column[ROWS - 1] < 100,
            "the top is dark: {}",
            column[ROWS - 1]
        );
        let named = analyser.column_note(3).unwrap();
        assert!((named - 1000.0).abs() < 3.0, "the column names {named} Hz");
        // A long stall writes no more than the tap still holds.
        analyser.step(&tap, start + HOP * 400);
        assert_eq!(analyser.take_fresh().len(), CATCH_UP);
        // After a pause the clock starts again with one column.
        analyser.pause();
        analyser.step(&tap, start + HOP * 900);
        assert_eq!(analyser.take_fresh().len(), 1);
    }

    #[test]
    fn peaks_hold_then_fall_but_never_below_the_spectrum() {
        let loud = tap_with(&sine(1000.0, 1.0, FFT_SIZE));
        let quiet = AudioTap::new();
        let mut analyser = ProAnalyser::default();
        let start = Instant::now();
        analyser.step(&loud, start);
        let bin = (1000.0 / bin_hz()).round() as usize;
        let top = analyser.peaks()[bin];
        analyser.step(&quiet, start + Duration::from_millis(200));
        assert_eq!(analyser.peaks()[bin], top, "held");
        assert!(analyser.spectrum()[bin] < top, "the spectrum fell");
        let mut at = start + Duration::from_millis(200);
        for _ in 0..40 {
            at += Duration::from_millis(100);
            analyser.step(&quiet, at);
            assert!(analyser.peaks()[bin] >= analyser.spectrum()[bin]);
        }
        assert!(analyser.peaks()[bin] < top - 10.0, "then fell");
    }

    #[test]
    fn the_scope_spans_its_time_base() {
        let mut analyser = ProAnalyser {
            time_base: 3,
            ..ProAnalyser::default()
        };
        assert_eq!(analyser.scope_span(), 441);
        analyser.set_time_base(TIME_BASES.len());
        assert_eq!(analyser.time_base(), TIME_BASES.len() - 1);
        assert_eq!(analyser.scope_span(), SCOPE_KEPT / 2);
        let (trace, triggered) = analyser.trace();
        assert_eq!(trace.len(), SCOPE_KEPT / 2);
        assert!(!triggered, "silence free-runs");
    }

    /// A wave louder than full scale still fits: the gain drops at once,
    /// then a quieter wave brings it back up over a second or so.
    #[test]
    fn the_fitted_gain_never_lets_the_wave_clip() {
        let loud = tap_with(&sine(441.0, 1.6, SCOPE_KEPT));
        let mut analyser = ProAnalyser::default();
        let start = Instant::now();
        analyser.step(&loud, start);
        let peak = analyser
            .trace()
            .0
            .iter()
            .fold(0.0f32, |most, sample| most.max(sample.abs()));
        assert!(
            peak * analyser.gain() <= AUTO_FILL + 1e-3,
            "{}",
            analyser.gain()
        );
        let quiet = tap_with(&sine(441.0, 0.1, SCOPE_KEPT));
        analyser.step(&quiet, start + Duration::from_millis(100));
        let rising = analyser.gain();
        assert!(rising < AUTO_FILL / 0.1, "it rises slowly: {rising}");
        let mut at = start + Duration::from_millis(100);
        for _ in 0..60 {
            at += Duration::from_millis(100);
            analyser.step(&quiet, at);
        }
        assert!(
            (analyser.gain() - AUTO_FILL / 0.1).abs() < 0.2,
            "{}",
            analyser.gain()
        );
        assert_eq!(analyser.trails().len(), TRAILS);
        analyser.set_gain_index(2);
        assert_eq!(analyser.gain(), 1.0);
    }

    /// Loudness reads every frame once, across steps, and a new song
    /// starts it afresh.
    #[test]
    fn loudness_follows_the_song() {
        let tap = AudioTap::new();
        let mut analyser = ProAnalyser::default();
        analyser.follow("spotify:track:one");
        let second = sine(997.0, 0.5, SAMPLE_RATE as usize / 4);
        let interleaved: Vec<f64> = second.iter().flat_map(|s| [f64::from(*s); 2]).collect();
        let start = Instant::now();
        for step in 0..12 {
            tap.push(&interleaved, 1.0);
            analyser.step(&tap, start + Duration::from_millis(250 * step));
        }
        let level = analyser.loudness().short_term().unwrap();
        assert!((level + 6.02).abs() < 0.3, "{level} LUFS");
        analyser.follow("spotify:track:one");
        assert!(analyser.loudness().integrated().is_some(), "same song");
        analyser.follow("spotify:track:two");
        assert_eq!(analyser.loudness().integrated(), None, "a new song");
    }
}
