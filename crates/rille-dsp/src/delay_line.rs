//! Stereo delay line with fractional reads.

/// Power-of-two ring buffer of stereo frames. Reads are relative to the most
/// recent write: `read(1.0)` is the last frame written.
#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Box<[[f32; 2]]>,
    mask: usize,
    pos: usize,
}

impl DelayLine {
    /// A line that can delay by up to `max_delay` frames.
    pub fn new(max_delay: usize) -> Self {
        let len = (max_delay + 2).next_power_of_two();
        Self { buf: vec![[0.0; 2]; len].into_boxed_slice(), mask: len - 1, pos: 0 }
    }

    /// Longest supported delay in frames.
    pub fn max_delay(&self) -> usize {
        self.mask - 1
    }

    pub fn clear(&mut self) {
        self.buf.fill([0.0; 2]);
    }

    #[inline]
    pub fn write(&mut self, x: [f32; 2]) {
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) & self.mask;
    }

    /// The frame written `delay` frames ago (clamped to `1..=max_delay`).
    #[inline]
    pub fn read_int(&self, delay: usize) -> [f32; 2] {
        let d = delay.clamp(1, self.max_delay());
        self.buf[self.pos.wrapping_sub(d) & self.mask]
    }

    /// Linearly interpolated read at a fractional delay (clamped to `1..=max_delay`).
    #[inline]
    pub fn read(&self, delay: f32) -> [f32; 2] {
        let d = delay.max(1.0).min(self.max_delay() as f32);
        let i = d as usize;
        let t = d - i as f32;
        let a = self.buf[self.pos.wrapping_sub(i) & self.mask];
        let b = self.buf[self.pos.wrapping_sub(i + 1) & self.mask];
        [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_and_fractional_reads() {
        let mut d = DelayLine::new(10);
        for i in 0..20 {
            d.write([i as f32, -(i as f32)]);
        }
        assert_eq!(d.read_int(1), [19.0, -19.0]);
        assert_eq!(d.read_int(5), [15.0, -15.0]);
        assert_eq!(d.read(2.25), [17.75, -17.75]);
        assert_eq!(d.read(0.0), [19.0, -19.0]);
        assert_eq!(d.read(f32::NAN), [19.0, -19.0]);
        assert!(d.max_delay() >= 10);
    }
}
