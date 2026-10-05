//! Loudness as ITU-R BS.1770-4 measures it. Each channel is K-weighted (a
//! high shelf for the head's effect, then a high-pass), the mean squares z
//! of the channels are summed, and loudness is L = -0.691 + 10·log₁₀(z)
//! LUFS. Momentary reads the last 400 ms, short-term the last 3 s, and
//! integrated the whole song through two gates: blocks under -70 LUFS are
//! silence, and blocks more than 10 LU under the mean of the rest are
//! pauses.

use std::collections::VecDeque;
use std::f64::consts::PI;

use librespot_playback::SAMPLE_RATE;

/// Frames in one 100 ms step, the hop between overlapping 400 ms blocks.
const STEP: usize = SAMPLE_RATE as usize / 10;
const MOMENTARY_STEPS: usize = 4;
const SHORT_TERM_STEPS: usize = 30;
const ABSOLUTE_GATE: f64 = -70.0;
const RELATIVE_GATE: f64 = -10.0;
/// Spotify's normalisation target, which most streaming services share.
pub const STREAMING_TARGET: f32 = -14.0;

/// One second-order section, in transposed direct form II.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// The two K-weighting stages at `rate`, derived from their analogue
/// prototypes as libebur128 does, so any sample rate gets the response
/// BS.1770 tabulates at 48 kHz.
fn k_weighting(rate: f64) -> [Biquad; 2] {
    let shelf = {
        let f0 = 1_681.974_450_955_533;
        let gain_db = 3.999_843_853_973_347;
        let q = 0.707_175_236_955_419_6;
        let k = (PI * f0 / rate).tan();
        let vh = 10f64.powf(gain_db / 20.0);
        let vb = vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        Biquad {
            b: [
                (vh + vb * k / q + k * k) / a0,
                2.0 * (k * k - vh) / a0,
                (vh - vb * k / q + k * k) / a0,
            ],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            z: [0.0; 2],
        }
    };
    let high_pass = {
        let f0 = 38.135_470_876_024_44;
        let q = 0.500_327_037_323_877_3;
        let k = (PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        Biquad {
            b: [1.0, -2.0, 1.0],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            z: [0.0; 2],
        }
    };
    [shelf, high_pass]
}

fn lufs(z: f64) -> f64 {
    -0.691 + 10.0 * z.log10()
}

/// A loudness meter over one song.
pub struct Meter {
    /// Each channel's two stages.
    filters: [[Biquad; 2]; 2],
    /// The current step's sum of squares over both channels, and its frames.
    sum: f64,
    frames: usize,
    /// The last thirty steps' summed mean squares, newest last.
    steps: VecDeque<f64>,
    /// Every 400 ms block's z since the song began.
    blocks: Vec<f64>,
    integrated: Option<f32>,
}

impl Default for Meter {
    fn default() -> Self {
        let filters = k_weighting(f64::from(SAMPLE_RATE));
        Self {
            filters: [filters, filters],
            sum: 0.0,
            frames: 0,
            steps: VecDeque::with_capacity(SHORT_TERM_STEPS),
            blocks: Vec::new(),
            integrated: None,
        }
    }
}

impl Meter {
    /// Measures `frames`, left and right, which follow the last ones given.
    pub fn push(&mut self, frames: &[[f32; 2]]) {
        for frame in frames {
            for (filters, sample) in self.filters.iter_mut().zip(frame) {
                let [shelf, high_pass] = filters;
                let weighted = high_pass.process(shelf.process(f64::from(*sample)));
                self.sum += weighted * weighted;
            }
            self.frames += 1;
            if self.frames == STEP {
                self.end_step();
            }
        }
    }

    fn end_step(&mut self) {
        let z = self.sum / STEP as f64;
        self.sum = 0.0;
        self.frames = 0;
        if self.steps.len() == SHORT_TERM_STEPS {
            self.steps.pop_front();
        }
        self.steps.push_back(z);
        if self.steps.len() >= MOMENTARY_STEPS {
            let block =
                self.steps.iter().rev().take(MOMENTARY_STEPS).sum::<f64>() / MOMENTARY_STEPS as f64;
            self.blocks.push(block);
            self.integrated = gated(&self.blocks);
        }
    }

    /// Loudness over the last `steps` steps, or as many as there are once
    /// there are 400 ms of them.
    fn over(&self, steps: usize) -> Option<f32> {
        if self.steps.len() < MOMENTARY_STEPS {
            return None;
        }
        let taken = steps.min(self.steps.len());
        let z = self.steps.iter().rev().take(taken).sum::<f64>() / taken as f64;
        (z > 1e-12).then(|| lufs(z) as f32)
    }

    /// The last 400 ms, in LUFS.
    pub fn momentary(&self) -> Option<f32> {
        self.over(MOMENTARY_STEPS)
    }

    /// The last 3 s, in LUFS.
    pub fn short_term(&self) -> Option<f32> {
        self.over(SHORT_TERM_STEPS)
    }

    /// The whole song so far, gated, in LUFS.
    pub fn integrated(&self) -> Option<f32> {
        self.integrated
    }
}

/// The mean of the blocks that pass both gates, in LUFS.
fn gated(blocks: &[f64]) -> Option<f32> {
    let audible = || {
        blocks
            .iter()
            .copied()
            .filter(|z| *z > 0.0 && lufs(*z) > ABSOLUTE_GATE)
    };
    let count = audible().count();
    if count == 0 {
        return None;
    }
    let threshold = lufs(audible().sum::<f64>() / count as f64) + RELATIVE_GATE;
    let (sum, kept) = audible()
        .filter(|z| lufs(*z) > threshold)
        .fold((0.0, 0usize), |(sum, kept), z| (sum + z, kept + 1));
    (kept > 0).then(|| lufs(sum / kept as f64) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f64, amplitude: f64, seconds: f64) -> Vec<f32> {
        let count = (seconds * f64::from(SAMPLE_RATE)) as usize;
        (0..count)
            .map(|i| (amplitude * (2.0 * PI * hz * i as f64 / f64::from(SAMPLE_RATE)).sin()) as f32)
            .collect()
    }

    /// BS.1770's own check: a full-scale 1 kHz sine in one channel reads
    /// -3.01 LUFS; in both, 0.
    #[test]
    fn a_full_scale_sine_reads_as_the_standard_says() {
        let wave = sine(997.0, 1.0, 2.0);
        let mut one = Meter::default();
        one.push(&wave.iter().map(|s| [*s, 0.0]).collect::<Vec<_>>());
        let level = one.momentary().unwrap();
        assert!((level + 3.01).abs() < 0.1, "{level} LUFS");
        let mut both = Meter::default();
        both.push(&wave.iter().map(|s| [*s, *s]).collect::<Vec<_>>());
        let level = both.short_term().unwrap();
        assert!(level.abs() < 0.1, "{level} LUFS");
        assert!((both.integrated().unwrap() - level).abs() < 0.1);
    }

    /// The relative gate leaves a quiet stretch out of the song's loudness.
    #[test]
    fn integrated_loudness_gates_out_the_quiet_part() {
        let mut meter = Meter::default();
        let loud = sine(997.0, 0.5, 3.0);
        let quiet = sine(997.0, 0.005, 3.0);
        meter.push(&loud.iter().map(|s| [*s, *s]).collect::<Vec<_>>());
        meter.push(&quiet.iter().map(|s| [*s, *s]).collect::<Vec<_>>());
        // The three blocks that straddle the change still pass the gate,
        // which takes a quarter of a decibel off.
        let integrated = meter.integrated().unwrap();
        assert!((integrated + 6.02).abs() < 0.3, "{integrated} LUFS");
        let momentary = meter.momentary().unwrap();
        assert!(momentary < -40.0, "{momentary} LUFS");
    }

    #[test]
    fn silence_has_no_loudness() {
        let mut meter = Meter::default();
        meter.push(&vec![[0.0, 0.0]; SAMPLE_RATE as usize]);
        assert_eq!(meter.momentary(), None);
        assert_eq!(meter.integrated(), None);
    }
}
