//! Parameter smoothers, so knob changes never produce zipper noise or clicks.

/// Ramps linearly to its target over a fixed time. Retargeting mid-ramp
/// starts a new ramp from the current value.
#[derive(Clone, Debug)]
pub struct LinearSmoother {
    value: f32,
    target: f32,
    step: f32,
    left: u32,
    ramp_frames: u32,
}

impl LinearSmoother {
    pub fn new(sample_rate: f32, ramp_ms: f32, initial: f32) -> Self {
        let ramp_frames = ((ramp_ms * 0.001 * sample_rate).round() as u32).max(1);
        Self { value: initial, target: initial, step: 0.0, left: 0, ramp_frames }
    }

    /// Starts a ramp to `target`. Non-finite targets are ignored.
    pub fn set_target(&mut self, target: f32) {
        if target == self.target || !target.is_finite() {
            return;
        }
        self.target = target;
        self.left = self.ramp_frames;
        self.step = (target - self.value) / self.ramp_frames as f32;
    }

    /// Jumps to `value` without a ramp.
    pub fn set_immediate(&mut self, value: f32) {
        if value.is_finite() {
            self.value = value;
            self.target = value;
            self.left = 0;
        }
    }

    /// Advances one frame and returns the new value.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            self.value = if self.left == 0 { self.target } else { self.value + self.step };
        }
        self.value
    }

    /// Advances `frames` frames at once and returns the new value.
    pub fn skip(&mut self, frames: usize) -> f32 {
        let n = frames.min(self.left as usize) as u32;
        self.left -= n;
        self.value = if self.left == 0 { self.target } else { self.value + self.step * n as f32 };
        self.value
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn is_settled(&self) -> bool {
        self.left == 0
    }
}

/// Exponential (one-pole low-pass) smoother. Snaps to the target once the
/// remaining step is lost in float precision, so it settles exactly and never
/// goes subnormal.
#[derive(Clone, Debug)]
pub struct OnePole {
    value: f32,
    target: f32,
    coef: f32,
}

impl OnePole {
    /// `time_ms` is the time constant (63 % of a step).
    pub fn new(sample_rate: f32, time_ms: f32, initial: f32) -> Self {
        let mut s = Self { value: initial, target: initial, coef: 1.0 };
        s.set_time(sample_rate, time_ms);
        s
    }

    pub fn set_time(&mut self, sample_rate: f32, time_ms: f32) {
        let frames = time_ms * 0.001 * sample_rate;
        self.coef = if frames > 1e-3 { 1.0 - (-1.0 / frames).exp() } else { 1.0 };
    }

    /// Sets the value approached by [`tick`](Self::tick). Non-finite targets are ignored.
    pub fn set_target(&mut self, target: f32) {
        if target.is_finite() {
            self.target = target;
        }
    }

    /// Jumps to `value` without smoothing.
    pub fn set_immediate(&mut self, value: f32) {
        if value.is_finite() {
            self.value = value;
            self.target = value;
        }
    }

    /// Advances one step towards the target and returns the new value.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        let d = self.target - self.value;
        let next = self.value + d * self.coef;
        // Snap once the step no longer changes the value (or is negligible).
        self.value = if next == self.value || d.abs() <= 1e-7 + self.target.abs() * 1e-6 { self.target } else { next };
        self.value
    }

    /// Low-pass filters `x`: sets it as the target and advances one step.
    #[inline]
    pub fn filter(&mut self, x: f32) -> f32 {
        self.set_target(x);
        self.tick()
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn is_settled(&self) -> bool {
        self.value == self.target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_reaches_target_exactly() {
        let mut s = LinearSmoother::new(48_000.0, 10.0, 0.0);
        s.set_target(1.0);
        let mut prev = 0.0;
        for _ in 0..479 {
            let v = s.tick();
            assert!(v > prev && v < 1.0);
            prev = v;
        }
        assert_eq!(s.tick(), 1.0);
        assert!(s.is_settled());
        s.set_target(0.0);
        assert_eq!(s.skip(1000), 0.0);
        s.set_target(f32::NAN);
        assert_eq!(s.tick(), 0.0);
    }

    #[test]
    fn linear_retarget_mid_ramp_is_continuous() {
        let mut s = LinearSmoother::new(1000.0, 10.0, 0.0);
        s.set_target(1.0);
        let a = s.skip(5);
        s.set_target(0.0);
        let b = s.tick();
        assert!((a - 0.5).abs() < 1e-6 && (a - b - 0.05).abs() < 1e-6);
    }

    #[test]
    fn one_pole_time_constant_and_settling() {
        let mut s = OnePole::new(48_000.0, 10.0, 0.0);
        s.set_target(1.0);
        let mut v = 0.0;
        for _ in 0..480 {
            v = s.tick();
        }
        assert!((v - (1.0 - (-1f32).exp())).abs() < 0.01, "{v}");
        for _ in 0..48_000 {
            s.tick();
        }
        assert!(s.is_settled());
        s.set_target(0.0);
        for _ in 0..48_000 {
            s.tick();
        }
        assert_eq!(s.value(), 0.0);
    }
}
