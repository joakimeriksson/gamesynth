//! Playing a sound faster or slower, for Doppler.
//!
//! A continuous generator has no single pitch to move: an engine is firing pulses through pipe
//! resonances, a tyre is filtered noise. Moving all of it together means reading its output at
//! another rate, which is what [`Resampler`] does. A ratio of 1.5 plays it half again as fast,
//! so every frequency in it, resonances and noise colour included, rises by 7 semitones.
//!
//! It costs nothing until the ratio first leaves 1: until then the source renders straight into
//! the output. Once engaged it stays engaged (until [`Resampler::reset`]), because dropping out
//! again would jump by a fraction of a sample. Ratio changes are ramped across each block.
//!
//! There is no anti-aliasing filter: at a ratio above 1, anything above `sr / 2 / ratio` folds
//! back down. For Doppler (0.8 to 1.25) that only touches the top 4 kHz of a 48 kHz stream,
//! where these sounds have next to nothing.

use crate::synth::MAX_BLOCK;

/// The ratio is clamped to this range. Godot allows 1/8 to 8; four times either way is more
/// than any Doppler shift and keeps the buffer small.
pub const MIN_RATIO: f32 = 0.25;
pub const MAX_RATIO: f32 = 4.0;

/// Source frames held: one output block at the highest ratio, plus interpolation context.
const CAPACITY: usize = MAX_BLOCK * MAX_RATIO as usize + 8;

/// Variable-rate reader for a stereo source. Allocates once, in [`Resampler::new`].
#[derive(Clone, Debug)]
pub struct Resampler {
    left: Vec<f32>,
    right: Vec<f32>,
    /// Source frames in the buffers.
    len: usize,
    /// Read position into the buffers. Always at least 1, so the frame before it is there.
    pos: f64,
    /// The ratio the last block ended on.
    ratio: f32,
    engaged: bool,
}

impl Default for Resampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Resampler {
    pub fn new() -> Self {
        Resampler { left: vec![0.0; CAPACITY], right: vec![0.0; CAPACITY], len: 0, pos: 1.0, ratio: 1.0, engaged: false }
    }

    /// Back to the pass-through state (for a playback that restarts).
    pub fn reset(&mut self) {
        self.len = 0;
        self.pos = 1.0;
        self.ratio = 1.0;
        self.engaged = false;
    }

    /// Whether it has left pass-through.
    pub fn engaged(&self) -> bool {
        self.engaged
    }

    /// Fill `left` and `right` (same length) with the source played at `ratio` times its speed.
    /// `render` fills its two slices with the next source frames; it is asked for at most
    /// [`MAX_BLOCK`] at a time.
    pub fn process(&mut self, ratio: f32, left: &mut [f32], right: &mut [f32], mut render: impl FnMut(&mut [f32], &mut [f32])) {
        debug_assert_eq!(left.len(), right.len());
        let target = if ratio.is_finite() { ratio.clamp(MIN_RATIO, MAX_RATIO) } else { 1.0 };
        if !self.engaged {
            if (target - 1.0).abs() < 1e-4 {
                for (l, r) in left.chunks_mut(MAX_BLOCK).zip(right.chunks_mut(MAX_BLOCK)) {
                    render(l, r);
                }
                return;
            }
            self.engaged = true;
            self.len = 0;
            self.pos = 1.0;
            self.ratio = 1.0;
        }
        for (l, r) in left.chunks_mut(MAX_BLOCK).zip(right.chunks_mut(MAX_BLOCK)) {
            self.block(target, l, r, &mut render);
        }
    }

    fn block(&mut self, target: f32, left: &mut [f32], right: &mut [f32], render: &mut impl FnMut(&mut [f32], &mut [f32])) {
        let n = left.len();
        let (from, step) = (self.ratio as f64, (target - self.ratio) as f64 / n as f64);
        // Where the read position ends up after this block (the ramp is linear, so its sum is too).
        let end = self.pos + n as f64 * (from + step * (n as f64 + 1.0) / 2.0);
        // The cubic reads one frame before the position and two after.
        let need = (end.floor() as usize + 3).min(CAPACITY);
        while self.len < need {
            let m = (need - self.len).min(MAX_BLOCK);
            render(&mut self.left[self.len..self.len + m], &mut self.right[self.len..self.len + m]);
            self.len += m;
        }
        let mut pos = self.pos;
        for (k, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let i = (pos.floor() as usize).min(self.len - 3);
            let f = (pos - i as f64) as f32;
            *l = cubic(&self.left[i - 1..i + 3], f);
            *r = cubic(&self.right[i - 1..i + 3], f);
            pos += from + step * (k + 1) as f64;
        }
        self.ratio = target;
        // Drop what has been read, keeping the frame before the position.
        let drop = (pos.floor() as usize).saturating_sub(1).min(self.len);
        self.left.copy_within(drop..self.len, 0);
        self.right.copy_within(drop..self.len, 0);
        self.len -= drop;
        self.pos = pos - drop as f64;
    }
}

/// Catmull-Rom interpolation between `y[1]` and `y[2]` at `t` in 0..1.
#[inline]
fn cubic(y: &[f32], t: f32) -> f32 {
    let (y0, y1, y2, y3) = (y[0], y[1], y[2], y[3]);
    let a = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
    let b = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c = 0.5 * (y2 - y0);
    ((a * t + b) * t + c) * t + y1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32) -> impl FnMut(&mut [f32], &mut [f32]) {
        let mut n = 0u64;
        move |l: &mut [f32], r: &mut [f32]| {
            for (a, b) in l.iter_mut().zip(r.iter_mut()) {
                *a = (core::f32::consts::TAU * hz * n as f32 / 48000.0).sin();
                *b = *a;
                n += 1;
            }
        }
    }

    #[test]
    fn interpolation_is_exact_on_the_samples() {
        assert_eq!(cubic(&[0.0, 1.0, 2.0, 3.0], 0.0), 1.0);
        assert!((cubic(&[0.0, 1.0, 2.0, 3.0], 0.5) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn a_long_ramp_keeps_the_buffer_bounded() {
        let mut rs = Resampler::new();
        let mut src = sine(440.0);
        let (mut l, mut r) = (vec![0.0; 1000], vec![0.0; 1000]);
        for k in 0..400 {
            let ratio = [0.25, 4.0, 1.0, 1.7][k % 4];
            rs.process(ratio, &mut l, &mut r, &mut src);
            assert!(rs.len <= CAPACITY && rs.pos >= 1.0 && rs.pos < rs.len as f64);
            assert!(l.iter().all(|x| x.is_finite() && x.abs() <= 1.3));
        }
    }
}
