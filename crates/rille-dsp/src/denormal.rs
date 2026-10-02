//! Denormal protection.
//!
//! Decaying filter and feedback states can drop into the subnormal range,
//! where x86 arithmetic becomes very slow. [`flush_denormals`] makes the CPU
//! treat them as zero; [`undenormal`] flushes feedback values explicitly so
//! the code is also safe where the CPU flag is not set.

/// Sets flush-to-zero and denormals-are-zero for the calling thread. Call once
/// at the start of the audio thread. No-op on targets other than x86_64.
pub fn flush_denormals() {
    #[cfg(target_arch = "x86_64")]
    {
        const FTZ: u32 = 1 << 15;
        const DAZ: u32 = 1 << 6;
        let mut csr: u32 = 0;
        // SAFETY: stores and reloads this thread's MXCSR through a valid local;
        // only the FTZ and DAZ bits change, which affects float results only
        // for subnormal values.
        unsafe {
            core::arch::asm!("stmxcsr [{}]", in(reg) &raw mut csr, options(nostack, preserves_flags));
            csr |= FTZ | DAZ;
            core::arch::asm!("ldmxcsr [{}]", in(reg) &raw const csr, options(nostack, preserves_flags));
        }
    }
}

/// Returns 0 for values too small to matter (including subnormals), else `x`.
#[inline(always)]
pub fn undenormal(x: f32) -> f32 {
    if x.abs() < 1e-18 { 0.0 } else { x }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undenormal_flushes_tiny() {
        assert_eq!(undenormal(f32::MIN_POSITIVE / 4.0), 0.0);
        assert_eq!(undenormal(1e-6), 1e-6);
        assert_eq!(undenormal(-0.5), -0.5);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn ftz_daz_set() {
        use std::hint::black_box;
        // Runs on its own test thread, so the flag does not leak into other tests.
        let sub = black_box(f32::MIN_POSITIVE) * black_box(0.25);
        assert!(sub > 0.0);
        flush_denormals();
        assert_eq!(black_box(f32::MIN_POSITIVE) * black_box(0.25), 0.0);
    }
}
