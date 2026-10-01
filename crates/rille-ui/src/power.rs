//! Laptop battery state from `/sys/class/power_supply`, so the header can
//! warn before the machine runs flat mid-set.
//!
//! Battery reads can go through the ACPI embedded controller and take tens
//! of milliseconds, so a background thread polls and the UI reads the last
//! result.

use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const SUPPLIES: &str = "/sys/class/power_supply";
const POLL: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Battery {
    /// Running from the battery (no charger connected).
    pub on_battery: bool,
    /// Charge 0..100.
    pub percent: f64,
    /// Estimated runtime left while discharging.
    pub minutes_left: Option<u32>,
}

static STATE: OnceLock<Mutex<Option<Battery>>> = OnceLock::new();

/// The latest battery state; `None` on machines without a system battery.
/// The first call starts the polling thread.
pub fn current() -> Option<Battery> {
    let state = STATE.get_or_init(|| {
        let first = read(Path::new(SUPPLIES));
        // Desktops have no system battery; nothing to poll.
        if first.is_some() {
            let _ = std::thread::Builder::new().name("battery".into()).spawn(|| {
                loop {
                    std::thread::sleep(POLL);
                    let b = read(Path::new(SUPPLIES));
                    if let Some(state) = STATE.get() {
                        *state.lock().unwrap_or_else(|e| e.into_inner()) = b;
                    }
                }
            });
        }
        Mutex::new(first)
    });
    *state.lock().unwrap_or_else(|e| e.into_inner())
}

fn attr(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string())
}

fn num(dir: &Path, name: &str) -> Option<f64> {
    attr(dir, name)?.parse().ok()
}

/// Reads all supplies under `root`. Peripheral batteries (wireless mice,
/// headsets; `scope` = "Device") are ignored.
fn read(root: &Path) -> Option<Battery> {
    let mut found = false;
    let mut charger_known = false;
    let mut charger_online = false;
    let mut discharging = false;
    // Energy (µWh) or charge (µAh) sums across batteries, and the drain rate
    // in the same unit per hour.
    let (mut now, mut full, mut rate) = (0.0, 0.0, 0.0);
    let mut capacities = Vec::new();

    for entry in fs::read_dir(root).ok()?.flatten() {
        let dir = entry.path();
        if attr(&dir, "scope").as_deref() == Some("Device") {
            continue;
        }
        if attr(&dir, "type").as_deref() != Some("Battery") {
            // Mains, USB, USB-C chargers.
            if let Some(online) = num(&dir, "online") {
                charger_known = true;
                charger_online |= online > 0.0;
            }
            continue;
        }
        if num(&dir, "present") == Some(0.0) {
            continue;
        }
        found = true;
        discharging |= attr(&dir, "status").as_deref() == Some("Discharging");
        if let Some(c) = num(&dir, "capacity") {
            capacities.push(c);
        }
        let (n, f, r) = match (num(&dir, "energy_now"), num(&dir, "energy_full")) {
            (Some(n), Some(f)) => (n, f, num(&dir, "power_now")),
            _ => match (num(&dir, "charge_now"), num(&dir, "charge_full")) {
                (Some(n), Some(f)) => (n, f, num(&dir, "current_now")),
                _ => continue,
            },
        };
        now += n;
        full += f;
        // Some drivers report the rate negative while discharging.
        rate += r.unwrap_or(0.0).abs();
    }
    if !found {
        return None;
    }

    let on_battery = if charger_known { !charger_online } else { discharging };
    let percent = if full > 0.0 {
        now / full * 100.0
    } else if !capacities.is_empty() {
        capacities.iter().sum::<f64>() / capacities.len() as f64
    } else {
        return None;
    };
    let minutes_left = (on_battery && rate > 0.0 && now > 0.0).then(|| (now / rate * 60.0).round() as u32);
    Some(Battery { on_battery, percent: percent.clamp(0.0, 100.0), minutes_left })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supply(root: &Path, name: &str, attrs: &[(&str, &str)]) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        for (k, v) in attrs {
            fs::write(dir.join(k), format!("{v}\n")).unwrap();
        }
    }

    fn root(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rille-power-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn desktop_with_wireless_mouse_has_no_battery() {
        let r = root("desktop");
        supply(
            &r,
            "hidpp_battery_0",
            &[("type", "Battery"), ("scope", "Device"), ("status", "Discharging"), ("capacity", "40")],
        );
        assert_eq!(read(&r), None);
    }

    #[test]
    fn unplugged_laptop_reports_charge_and_runtime() {
        let r = root("unplugged");
        supply(&r, "AC", &[("type", "Mains"), ("online", "0")]);
        supply(
            &r,
            "BAT0",
            &[
                ("type", "Battery"),
                ("status", "Discharging"),
                ("energy_now", "30000000"),
                ("energy_full", "60000000"),
                ("power_now", "15000000"),
            ],
        );
        let b = read(&r).unwrap();
        assert!(b.on_battery);
        assert!((b.percent - 50.0).abs() < 1e-9);
        assert_eq!(b.minutes_left, Some(120));
    }

    #[test]
    fn plugged_in_laptop_is_not_on_battery() {
        let r = root("plugged");
        supply(&r, "AC", &[("type", "Mains"), ("online", "1")]);
        // At a charge threshold the battery neither charges nor discharges.
        supply(&r, "BAT0", &[("type", "Battery"), ("status", "Not charging"), ("capacity", "80")]);
        let b = read(&r).unwrap();
        assert!(!b.on_battery);
        assert_eq!(b.minutes_left, None);
    }

    #[test]
    fn two_batteries_combine_and_charge_units_work() {
        let r = root("dual");
        supply(&r, "ucsi-source-psy-USBC000:001", &[("type", "USB"), ("online", "0")]);
        supply(
            &r,
            "BAT0",
            &[
                ("type", "Battery"),
                ("status", "Discharging"),
                ("charge_now", "1000000"),
                ("charge_full", "4000000"),
                ("current_now", "-1000000"),
            ],
        );
        supply(
            &r,
            "BAT1",
            &[
                ("type", "Battery"),
                ("status", "Unknown"),
                ("charge_now", "3000000"),
                ("charge_full", "4000000"),
                ("current_now", "0"),
            ],
        );
        let b = read(&r).unwrap();
        assert!(b.on_battery);
        assert!((b.percent - 50.0).abs() < 1e-9);
        assert_eq!(b.minutes_left, Some(240));
    }

    #[test]
    fn no_charger_entry_falls_back_to_status() {
        let r = root("nocharger");
        supply(&r, "BAT0", &[("type", "Battery"), ("status", "Discharging"), ("capacity", "12")]);
        let b = read(&r).unwrap();
        assert!(b.on_battery);
        assert_eq!(b.percent, 12.0);
    }
}
