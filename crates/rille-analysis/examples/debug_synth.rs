//! Prints the grid report for a few synthetic tracks (development aid).

use rille_analysis::synth::{self, Spec};
use rille_analysis::{AnalysisConfig, analyze};

fn main() {
    let specs = [
        (
            "124 at 48k",
            Spec { sr: 48_000, sections: vec![(80, 124.0)], first_beat_secs: 1.237, seed: 2, ..Spec::default() },
        ),
        ("breakdown", Spec { sections: vec![(96, 127.0)], kickless_bars: Some((40, 56)), seed: 11, ..Spec::default() }),
        ("tempo change", Spec { sections: vec![(48, 120.0), (48, 126.0)], seed: 21, ..Spec::default() }),
        (
            "live",
            Spec { sections: vec![(64, 118.0)], jitter_ms: 4.0, drift_per_bar: 0.004, seed: 31, ..Spec::default() },
        ),
    ];
    for (name, spec) in specs {
        let r = synth::render(&spec);
        let out = analyze(&r.audio, &AnalysisConfig::default());
        println!("{name}: {:#?}", out.report);
    }
}
