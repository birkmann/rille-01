//! `rille-cli beatport …`: the app's Beatport client from the command line,
//! to check the sign-in and the API with a real account. Uses the app's
//! sign-in (`RILLE_PROFILE` or the XDG data folder).

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use rille_beatport::{Client, Quality, Track};

const USAGE: &str = "usage: rille-cli beatport login <user> | logout | whoami | search <text> | list <beatport url>
       | playlists | playlist <id> | fetch <track id> <out dir> [lossless|high|medium] | raw <api path>";

pub fn run(args: &[String]) {
    let client = Client::new(token_path());
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_else(|| fail(USAGE));
    let result = match args.first().map(String::as_str) {
        Some("login") => {
            let user = arg(1);
            let password = read_password();
            client.login(user, &password).map(|()| println!("signed in as {user}"))
        }
        Some("logout") => {
            client.logout();
            println!("signed out");
            Ok(())
        }
        Some("whoami") => {
            println!("{}", client.account().unwrap_or_else(|| "not signed in".into()));
            Ok(())
        }
        Some("search") => client.search(&args[1..].join(" ")).map(|t| print_tracks(&t)),
        Some("list") => match rille_beatport::parse_link(arg(1)) {
            Ok(link) => client.link_tracks(link).map(|t| print_tracks(&t)),
            Err(e) => fail(&e),
        },
        Some("playlists") => client.my_playlists().map(|lists| {
            for p in lists {
                println!("{:>10}  {:>4} tracks  {}", p.id, p.track_count.unwrap_or(0), p.name);
            }
        }),
        Some("playlist") => {
            client.playlist_tracks(arg(1).parse().unwrap_or_else(|_| fail(USAGE))).map(|t| print_tracks(&t))
        }
        // Debugging: any API path, e.g. `raw /catalog/genres/?per_page=5`.
        Some("raw") => client.raw(arg(1)).map(|v| println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default())),
        Some("fetch") => {
            let id: i64 = arg(1).parse().unwrap_or_else(|_| fail(USAGE));
            let dir = PathBuf::from(arg(2));
            let quality = Quality::from_name(args.get(3).map_or("lossless", String::as_str));
            client.download_location(id, quality).and_then(|d| {
                let dest = dir.join(format!("{id}.{}", d.extension()));
                eprintln!("{} → {}", d.stream_quality, dest.display());
                let mut shown = -1;
                let started = std::time::Instant::now();
                let bytes = client.fetch_file(
                    &d.location,
                    &dest,
                    &AtomicBool::new(false),
                    &mut |p| {
                        let pct = p.fraction().map_or(0, |f| (f * 100.0) as i32);
                        if pct / 10 != shown / 10 {
                            shown = pct;
                            eprint!("{pct}% ");
                        }
                    },
                    None,
                )?;
                eprintln!();
                let secs = started.elapsed().as_secs_f64();
                println!(
                    "{} ({:.1} MB in {secs:.1} s, {:.1} MB/s)",
                    dest.display(),
                    bytes as f64 / 1_048_576.0,
                    bytes as f64 / 1_048_576.0 / secs.max(0.001)
                );
                Ok(())
            })
        }
        _ => fail(USAGE),
    };
    if let Err(e) = result {
        fail(&e.to_string());
    }
}

fn print_tracks(tracks: &[Track]) {
    for t in tracks {
        let key = t.key.as_ref().and_then(|k| k.camelot()).unwrap_or_default();
        let bpm = t.bpm.map_or(String::new(), |b| format!("{b:.0}"));
        println!(
            "{:>10}  {:>3} {:>3}  {} – {}  [{}]",
            t.id,
            bpm,
            key,
            t.artist_names(),
            t.display_title(),
            t.label_name()
        );
    }
    eprintln!("{} tracks", tracks.len());
}

/// The password from stdin, not echoed on a terminal.
fn read_password() -> String {
    let tty = std::io::stdin().is_terminal();
    let stty = |arg: &str| {
        let _ = std::process::Command::new("stty").arg(arg).stdin(std::process::Stdio::inherit()).status();
    };
    if tty {
        eprint!("Beatport password: ");
        let _ = std::io::stderr().flush();
        stty("-echo");
    }
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    if tty {
        stty("echo");
        eprintln!();
    }
    if read.is_err() {
        fail("cannot read the password");
    }
    line.trim_end_matches(['\r', '\n']).to_owned()
}

fn token_path() -> PathBuf {
    match std::env::var_os("RILLE_PROFILE") {
        Some(dir) => rille_app::Paths::under(&PathBuf::from(dir)).beatport_token(),
        None => rille_app::Paths::xdg().beatport_token(),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}
