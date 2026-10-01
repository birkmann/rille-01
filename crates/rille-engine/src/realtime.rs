//! Real-time priority for the audio thread. Without it the thread competes
//! with every other program; when the machine is busy (a compile, an
//! analysis run) it misses its deadline and the output crackles.
//!
//! Desktop Linux grants real-time scheduling through RealtimeKit (the
//! service PipeWire itself uses): the process caps its real-time CPU time
//! with `RLIMIT_RTTIME`, then asks RealtimeKit over D-Bus to switch the
//! thread to `SCHED_RR`. The request goes through `busctl`, so no D-Bus
//! library is needed; it runs on a helper thread, never on the audio thread.
//!
//! Other platforms get no-op fallbacks with the same signatures.

#[cfg(target_os = "linux")]
use std::process::Command;

/// Kernel id of the calling thread (what RealtimeKit and `ps -L` call TID).
#[cfg(target_os = "linux")]
pub fn current_tid() -> i64 {
    // SAFETY: gettid has no preconditions and cannot fail.
    i64::from(unsafe { libc::gettid() })
}

#[cfg(target_os = "linux")]
const RTKIT: [&str; 3] =
    ["org.freedesktop.RealtimeKit1", "/org/freedesktop/RealtimeKit1", "org.freedesktop.RealtimeKit1"];

#[cfg(target_os = "linux")]
fn rtkit_property(name: &str) -> Option<i64> {
    let out =
        Command::new("busctl").args(["get-property", "--system", RTKIT[0], RTKIT[1], RTKIT[2], name]).output().ok()?;
    // Output looks like "i 20" or "x 200000".
    String::from_utf8_lossy(&out.stdout).split_whitespace().nth(1)?.parse().ok()
}

/// Makes thread `tid` of this process real-time. Returns the priority it got.
#[cfg(target_os = "linux")]
pub fn promote(tid: i64) -> Result<i64, String> {
    let prio = rtkit_property("MaxRealtimePriority").ok_or("RealtimeKit not available")?.clamp(1, 20);
    let rttime = rtkit_property("RTTimeUSecMax").unwrap_or(200_000).max(1);
    // RealtimeKit only serves processes that limit their real-time CPU time.
    let limit = libc::rlimit { rlim_cur: rttime as libc::rlim_t, rlim_max: rttime as libc::rlim_t };
    // SAFETY: plain syscall on a valid, initialized struct.
    if unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &limit) } != 0 {
        return Err("cannot set RLIMIT_RTTIME".into());
    }
    let pid = std::process::id().to_string();
    let out = Command::new("busctl")
        .args(["call", "--system", RTKIT[0], RTKIT[1], RTKIT[2], "MakeThreadRealtimeWithPID", "ttu"])
        .args([pid.as_str(), &tid.to_string(), &prio.to_string()])
        .output()
        .map_err(|e| format!("busctl: {e}"))?;
    if out.status.success() { Ok(prio) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

/// Scheduling policy of thread `tid` ("SCHED_RR", "SCHED_OTHER", …).
#[cfg(target_os = "linux")]
pub fn policy(tid: i64) -> &'static str {
    // SAFETY: plain syscall; an unknown tid returns −1.
    match unsafe { libc::sched_getscheduler(tid as libc::pid_t) } {
        libc::SCHED_FIFO => "SCHED_FIFO",
        libc::SCHED_RR => "SCHED_RR",
        libc::SCHED_OTHER => "SCHED_OTHER",
        _ => "unknown",
    }
}

/// Lowers the calling thread's priority (background work such as analysis),
/// so it never competes with playback or the UI.
#[cfg(target_os = "linux")]
pub fn lower_current_thread() {
    // SAFETY: plain syscall; `who` is this thread's id.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, current_tid() as libc::id_t, 10);
    }
}

#[cfg(not(target_os = "linux"))]
pub fn current_tid() -> i64 {
    0
}

#[cfg(not(target_os = "linux"))]
pub fn promote(_tid: i64) -> Result<i64, String> {
    Err("real-time priority not supported on this platform".into())
}

#[cfg(not(target_os = "linux"))]
pub fn policy(_tid: i64) -> &'static str {
    "SCHED_OTHER"
}

#[cfg(not(target_os = "linux"))]
pub fn lower_current_thread() {}
