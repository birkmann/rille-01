//! `cargo run --release -p rille-decode --example decode_bench -- <files...>`

use std::time::Instant;

fn main() {
    for arg in std::env::args().skip(1) {
        let t = Instant::now();
        match rille_decode::decode_file(arg.as_ref(), None, &mut |_| {}) {
            Ok(a) => {
                let el = t.elapsed().as_secs_f64();
                println!(
                    "{:7.3}s  {:6.1}x  {} Hz  {:7.2}s  {}",
                    el,
                    a.duration_secs() / el,
                    a.sample_rate,
                    a.duration_secs(),
                    arg
                );
            }
            Err(e) => println!("ERROR {e}: {arg}"),
        }
    }
}
