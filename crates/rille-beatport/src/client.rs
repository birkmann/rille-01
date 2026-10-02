//! The signed-in API client: catalog lists and track downloads.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::de::DeserializeOwned;
use ureq::Agent;

use crate::arrived::{Arrived, Outcome};
use crate::auth::{self, Token};
use crate::links::Link;
use crate::model::{Download, Named, Paginated, Playlist, PlaylistItem, SearchResults, Track};
use crate::{API, Error, Result, api_error, net};

/// Lists stop after this many tracks.
const MAX_TRACKS: usize = 500;
const PER_PAGE: usize = 100;
/// Parallel connections per download.
const PARTS: u64 = 8;
/// Smaller files (and parts) are not worth splitting further.
const MIN_PART: u64 = 2 << 20;

/// How far a download is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    /// 0 while unknown.
    pub total: u64,
}

impl Progress {
    /// 0..1, `None` while the size is unknown.
    pub fn fraction(self) -> Option<f32> {
        (self.total > 0).then(|| (self.done as f64 / self.total as f64).min(1.0) as f32)
    }
}

/// The ranges a file of `total` bytes downloads in, in order: small at the
/// start, so a track can play before it is all there, larger further on
/// (fewer requests), and small again at the end, so no connection idles
/// while the last one finishes.
fn chunks(total: u64) -> Vec<std::ops::Range<u64>> {
    const SMALLEST: u64 = 256 << 10;
    const LARGEST: u64 = 4 << 20;
    let mut out = Vec::new();
    let mut at = 0;
    while at < total {
        let size = (at / 4).clamp(SMALLEST, LARGEST).min((total - at) / PARTS).max(SMALLEST);
        let end = (at + size).min(total);
        out.push(at..end);
        at = end;
    }
    out
}

/// The full size from a `Content-Range: bytes 0-0/12345` header.
fn content_range_total(res: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    res.headers().get("content-range")?.to_str().ok()?.rsplit('/').next()?.trim().parse().ok()
}

/// Audio quality to stream. Lossless and AAC 256 need a Professional plan,
/// AAC 128 an Advanced one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    #[default]
    Lossless,
    High,
    Medium,
}

impl Quality {
    pub fn name(self) -> &'static str {
        match self {
            Self::Lossless => "lossless",
            Self::High => "high",
            Self::Medium => "medium",
        }
    }

    pub fn from_name(s: &str) -> Self {
        [Self::Lossless, Self::High, Self::Medium].into_iter().find(|q| q.name() == s).unwrap_or_default()
    }
}

pub struct Client {
    /// API calls; never follows redirects (sign-in reads them).
    api: Agent,
    /// Audio files and images, wherever they redirect to.
    files: Agent,
    token_path: PathBuf,
    token: Mutex<Option<Token>>,
}

impl Client {
    /// A client using the tokens saved at `token_path`, if any.
    pub fn new(token_path: PathBuf) -> Self {
        let api = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(40)))
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .into();
        let files = Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(40)))
            .timeout_recv_body(Some(Duration::from_secs(15 * 60)))
            .http_status_as_error(false)
            .build()
            .into();
        let token = Token::load(&token_path);
        Self { api, files, token_path, token: Mutex::new(token) }
    }

    /// The signed-in user name, `None` when signed out.
    pub fn account(&self) -> Option<String> {
        self.token.lock().expect("token lock").as_ref().map(|t| t.username.clone())
    }

    /// Signs in and remembers the tokens (not the password).
    pub fn login(&self, username: &str, password: &str) -> Result<()> {
        let token = auth::login(&self.api, username.trim(), password)?;
        token.save(&self.token_path)?;
        *self.token.lock().expect("token lock") = Some(token);
        Ok(())
    }

    pub fn logout(&self) {
        *self.token.lock().expect("token lock") = None;
        let _ = std::fs::remove_file(&self.token_path);
    }

    /// A current access token, refreshed first when it is about to expire
    /// (or when `force`d after the API refused it).
    fn access_token(&self, force: bool) -> Result<String> {
        let mut guard = self.token.lock().expect("token lock");
        let token = guard.as_ref().ok_or(Error::NotSignedIn)?;
        if !force && !token.expiring(auth::now()) {
            return Ok(token.access_token.clone());
        }
        match auth::refresh(&self.api, token) {
            Ok(new) => {
                let _ = new.save(&self.token_path);
                let access = new.access_token.clone();
                *guard = Some(new);
                Ok(access)
            }
            // The refresh token is no good any more: sign in again.
            Err(Error::Http { status: 400 | 401 | 403, .. }) => {
                *guard = None;
                let _ = std::fs::remove_file(&self.token_path);
                Err(Error::SessionExpired)
            }
            Err(e) => Err(e),
        }
    }

    /// GET an API path (below `/v4`) as JSON, retrying once with a fresh
    /// token if the API refuses the current one.
    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        let call = |token: &str| {
            let mut req = crate::with_headers(self.api.get(format!("{API}{path}")))
                .header("authorization", format!("Bearer {token}"));
            for (k, v) in query {
                req = req.query(*k, v);
            }
            req.call().map_err(net)
        };
        let mut res = call(&self.access_token(false)?)?;
        if res.status().as_u16() == 401 {
            res = call(&self.access_token(true)?)?;
        }
        if !res.status().is_success() {
            return Err(api_error(res));
        }
        res.body_mut().read_json().map_err(|e| Error::Decode(format!("{path}: {e}")))
    }

    /// All pages of a list, up to [`MAX_TRACKS`] entries.
    fn paged<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>> {
        let mut out = Vec::new();
        for page in 1.. {
            let p: Paginated<T> = self.get(path, &[("page", page.to_string()), ("per_page", PER_PAGE.to_string())])?;
            let n = p.results.len();
            out.extend(p.results);
            if p.next.is_none() || n == 0 || out.len() >= MAX_TRACKS {
                break;
            }
        }
        out.truncate(MAX_TRACKS);
        Ok(out)
    }

    /// Tracks available for streaming, best match first. Without `type`
    /// the API mixes in releases and artists and returns only a few tracks.
    pub fn search(&self, query: &str) -> Result<Vec<Track>> {
        let r: SearchResults = self.get(
            "/catalog/search/",
            &[
                ("q", query.trim().to_owned()),
                ("type", "tracks".into()),
                ("per_page", PER_PAGE.to_string()),
                ("is_available_for_streaming", "true".into()),
            ],
        )?;
        Ok(r.tracks)
    }

    /// Beatport's Top 100, overall or of one genre.
    pub fn top100(&self, genre: Option<i64>) -> Result<Vec<Track>> {
        match genre {
            Some(id) => self.paged(&format!("/catalog/genres/{id}/top/100/")),
            None => self.paged("/catalog/tracks/top/100/"),
        }
    }

    /// The genres, alphabetically.
    pub fn genres(&self) -> Result<Vec<Named>> {
        let mut g: Vec<Named> = self.paged("/catalog/genres/")?;
        g.sort_by_key(|n| n.name.to_lowercase());
        Ok(g)
    }

    /// Tracks the user bought, newest purchase first; only those that can
    /// be streamed (the others need the store's download).
    pub fn purchases(&self) -> Result<Vec<Track>> {
        let all: Vec<Track> = self.paged("/my/downloads/")?;
        Ok(all.into_iter().filter(|t| t.is_available_for_streaming != Some(false)).collect())
    }

    pub fn track(&self, id: i64) -> Result<Track> {
        self.get(&format!("/catalog/tracks/{id}/"), &[])
    }

    /// Any API path below `/v4` (with its query string) as JSON, for
    /// debugging.
    pub fn raw(&self, path: &str) -> Result<serde_json::Value> {
        self.get(path, &[])
    }

    /// The tracks a link points at: the track itself, a release, chart or
    /// playlist, or the top 100 of a label, artist or genre.
    pub fn link_tracks(&self, link: Link) -> Result<Vec<Track>> {
        match link {
            Link::Track(id) => Ok(vec![self.track(id)?]),
            Link::Release(id) => self.paged(&format!("/catalog/releases/{id}/tracks/")),
            Link::Chart(id) => self.paged(&format!("/catalog/charts/{id}/tracks/")),
            Link::Playlist(id) => self.playlist_tracks(id),
            Link::Label(id) => self.paged(&format!("/catalog/labels/{id}/top/100/")),
            Link::Artist(id) => self.paged(&format!("/catalog/artists/{id}/top/100/")),
            Link::Genre(id) => self.paged(&format!("/catalog/genres/{id}/top/100/")),
        }
    }

    /// The playlists in the user's Beatport library.
    pub fn my_playlists(&self) -> Result<Vec<Playlist>> {
        self.paged("/my/playlists/")
    }

    pub fn playlist_tracks(&self, id: i64) -> Result<Vec<Track>> {
        let mut items: Vec<PlaylistItem> = self.paged(&format!("/catalog/playlists/{id}/tracks/"))?;
        items.sort_by_key(|i| i.position.unwrap_or(u32::MAX));
        Ok(items.into_iter().filter_map(|i| i.track).collect())
    }

    /// Where to download a track's audio in `quality`.
    pub fn download_location(&self, id: i64, quality: Quality) -> Result<Download> {
        let d: Download =
            self.get(&format!("/catalog/tracks/{id}/download/"), &[("quality", quality.name().into())])?;
        if d.location.is_empty() {
            return Err(Error::Decode("download without a location".into()));
        }
        Ok(d)
    }

    /// Downloads `url` to `dest` (through `dest.part`, so `dest` only ever
    /// holds a whole file). Beatport's file server limits the speed of each
    /// connection, so [`PARTS`] connections fetch the file's [`chunks`] at
    /// once (measured: 8 parts take a third of the time of one). `progress` is called about
    /// ten times a second. `arrived` follows the ranges, to read the file
    /// while it comes (see [`Arrived`]).
    pub fn fetch_file(
        &self,
        url: &str,
        dest: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
        arrived: Option<&Arrived>,
    ) -> Result<u64> {
        let result = self.fetch_file_to(url, dest, cancel, progress, arrived);
        if let Some(a) = arrived {
            a.finish(if result.is_ok() { Outcome::Complete } else { Outcome::Failed });
        }
        result
    }

    fn fetch_file_to(
        &self,
        url: &str,
        dest: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
        arrived: Option<&Arrived>,
    ) -> Result<u64> {
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let part = dest.with_extension("part");
        match self.fetch_to(url, &part, cancel, progress, arrived) {
            Ok(n) => {
                std::fs::rename(&part, dest)?;
                Ok(n)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                Err(e)
            }
        }
    }

    fn fetch_to(
        &self,
        url: &str,
        path: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
        arrived: Option<&Arrived>,
    ) -> Result<u64> {
        // The first byte tells the size, and whether ranges work at all.
        let probe = crate::with_headers(self.files.get(url)).header("range", "bytes=0-0").call().map_err(net)?;
        let total = (probe.status().as_u16() == 206).then(|| content_range_total(&probe)).flatten();
        if !probe.status().is_success() {
            return Err(api_error(probe));
        }
        drop(probe);
        let Some(total) = total.filter(|t| *t >= 2 * MIN_PART) else {
            return self.fetch_whole(url, path, cancel, progress, arrived);
        };
        let file = std::fs::File::create(path)?;
        file.set_len(total)?;
        let chunks = chunks(total);
        if let Some(a) = arrived {
            a.begin(path.to_owned(), total, &chunks);
        }
        let parts = PARTS.min(chunks.len() as u64);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let done = AtomicU64::new(0);
        let running = AtomicU64::new(parts);
        // The first failure stops the other parts.
        let failed: Mutex<Option<Error>> = Mutex::new(None);
        let stop = AtomicBool::new(false);
        std::thread::scope(|s| {
            // Each connection takes the next chunk in file order, so the
            // start of the file is there first.
            for _ in 0..parts {
                let (file, done, running, failed, stop) = (&file, &done, &running, &failed, &stop);
                let (chunks, next) = (&chunks, &next);
                s.spawn(move || {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(range) = chunks.get(i) else { break };
                        let part = arrived.map(|a| (a, i));
                        if let Err(e) = self.fetch_range(url, file, range.clone(), done, cancel, stop, part) {
                            stop.store(true, Ordering::Relaxed);
                            failed.lock().expect("failed lock").get_or_insert(e);
                            break;
                        }
                    }
                    running.fetch_sub(1, Ordering::Relaxed);
                });
            }
            loop {
                progress(Progress { done: done.load(Ordering::Relaxed), total });
                if running.load(Ordering::Relaxed) == 0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        if let Some(e) = failed.into_inner().expect("failed lock") {
            return Err(e);
        }
        let got = done.load(Ordering::Relaxed);
        if got != total {
            return Err(Error::Net(format!("download ended after {got} of {total} bytes")));
        }
        file.sync_all()?;
        Ok(total)
    }

    /// One range of the file, written in place. A dropped connection resumes
    /// where it stopped (a few times). `part` reports each write.
    #[allow(clippy::too_many_arguments)]
    fn fetch_range(
        &self,
        url: &str,
        file: &std::fs::File,
        range: std::ops::Range<u64>,
        done: &AtomicU64,
        cancel: &AtomicBool,
        stop: &AtomicBool,
        part: Option<(&Arrived, usize)>,
    ) -> Result<()> {
        use std::os::unix::fs::FileExt;
        let mut pos = range.start;
        let mut failures = 0;
        let mut buf = vec![0u8; 256 * 1024];
        while pos < range.end {
            let before = pos;
            let req =
                crate::with_headers(self.files.get(url)).header("range", format!("bytes={pos}-{}", range.end - 1));
            let error = match req.call() {
                Ok(res) if res.status().as_u16() == 206 => {
                    let mut reader = res.into_body().into_reader();
                    loop {
                        if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
                            return Err(Error::Cancelled);
                        }
                        match reader.read(&mut buf) {
                            Ok(0) => break None,
                            Ok(n) => {
                                let n = n.min((range.end - pos) as usize);
                                file.write_all_at(&buf[..n], pos)?;
                                pos += n as u64;
                                done.fetch_add(n as u64, Ordering::Relaxed);
                                if let Some((a, i)) = part {
                                    a.advance(i, pos);
                                }
                                if pos >= range.end {
                                    break None;
                                }
                            }
                            Err(e) => break Some(Error::Net(e.to_string())),
                        }
                    }
                }
                Ok(res) => return Err(api_error(res)),
                Err(e) => Some(net(e)),
            };
            if pos == before {
                failures += 1;
                if failures > 3 {
                    return Err(error.unwrap_or_else(|| Error::Net("the download stopped".into())));
                }
            }
        }
        Ok(())
    }

    /// Without ranges: one connection.
    fn fetch_whole(
        &self,
        url: &str,
        path: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
        arrived: Option<&Arrived>,
    ) -> Result<u64> {
        let res = crate::with_headers(self.files.get(url)).call().map_err(net)?;
        if !res.status().is_success() {
            return Err(api_error(res));
        }
        let total = res.body().content_length().unwrap_or(0);
        let mut file = std::fs::File::create(path)?;
        // Readable while it comes only with a known size.
        let arrived = arrived.filter(|_| total > 0);
        if let Some(a) = arrived {
            a.begin(path.to_owned(), total, std::slice::from_ref(&(0..total)));
        }
        let mut reader = res.into_body().into_reader();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        let mut shown = std::time::Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let n = reader.read(&mut buf).map_err(|e| Error::Net(e.to_string()))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            done += n as u64;
            if let Some(a) = arrived {
                a.advance(0, done);
            }
            if shown.elapsed().as_millis() >= 100 {
                shown = std::time::Instant::now();
                progress(Progress { done, total });
            }
        }
        if total > 0 && total != done {
            return Err(Error::Net(format!("download ended after {done} of {total} bytes")));
        }
        progress(Progress { done, total: total.max(done) });
        file.sync_all()?;
        Ok(done)
    }

    /// A cover image (JPEG or PNG), up to `limit` bytes. The image server
    /// sends WebP to anyone who accepts it, which rille cannot read.
    pub fn fetch_image(&self, url: &str, limit: u64) -> Result<Vec<u8>> {
        let mut res =
            crate::with_accept(self.files.get(url), "image/jpeg,image/png;q=0.9,*/*;q=0.1").call().map_err(net)?;
        if !res.status().is_success() {
            return Err(api_error(res));
        }
        res.body_mut().with_config().limit(limit).read_to_vec().map_err(net)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_cover_the_file_small_at_both_ends() {
        let total = 52_482_667;
        let c = chunks(total);
        assert_eq!(c.first().map(|r| r.start), Some(0));
        assert_eq!(c.last().map(|r| r.end), Some(total));
        assert!(c.windows(2).all(|w| w[0].end == w[1].start));
        assert!(c.iter().take(4).all(|r| r.end - r.start == 256 << 10), "{:?}", &c[..4]);
        assert!(c.iter().any(|r| r.end - r.start > 3 << 20), "{c:?}");
        assert!(c.last().is_some_and(|r| r.end - r.start <= 256 << 10));
        assert!(c.len() < 50, "{} requests", c.len());
        assert_eq!(chunks(10).first(), Some(&(0..10)));
        assert_eq!(chunks(10).len(), 1);
    }

    #[test]
    fn signed_out() {
        let c = Client::new(std::env::temp_dir().join("rille-bp-no-such-token.json"));
        assert_eq!(c.account(), None);
        assert!(matches!(c.search("x"), Err(Error::NotSignedIn)));
        assert_eq!(Quality::from_name("high"), Quality::High);
        assert_eq!(Quality::from_name("bogus"), Quality::Lossless);
    }

    /// A file server for one file that answers byte ranges, slowly (`delay`
    /// per 64 KiB written), like Beatport's.
    fn serve(body: Vec<u8>, delay: Duration) -> String {
        use std::io::BufRead;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/track.flac", listener.local_addr().unwrap());
        let body = std::sync::Arc::new(body);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let (Ok(mut stream), body) = (stream, body.clone()) else { continue };
                std::thread::spawn(move || {
                    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut range = None;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some(r) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            let (a, b) = r.trim().split_once('-').unwrap();
                            range = Some((a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()));
                        }
                    }
                    let (a, b) = range.unwrap_or((0, body.len() - 1));
                    let head = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {a}-{b}/{}\r\nConnection: close\r\n\r\n",
                        b + 1 - a,
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    for chunk in body[a..=b].chunks(64 * 1024) {
                        if stream.write_all(chunk).is_err() {
                            return;
                        }
                        std::thread::sleep(delay);
                    }
                });
            }
        });
        url
    }

    #[test]
    fn a_file_reads_while_it_downloads() {
        use crate::arrived::Arrived;
        let dir = tempfile::tempdir().unwrap();
        let body: Vec<u8> = (0..6 * MIN_PART as usize).map(|i| (i * 7 % 251) as u8).collect();
        // 8 MiB per second and range: the whole file in about 1.5 s.
        let url = serve(body.clone(), Duration::from_millis(8));
        let client = Client::new(dir.path().join("token.json"));
        let arrived = Arrived::new();
        let dest = dir.path().join("t.flac");
        let reader = std::thread::scope(|s| {
            let download =
                s.spawn(|| client.fetch_file(&url, &dest, &AtomicBool::new(false), &mut |_| {}, Some(&arrived)));
            let (part, total) = arrived.started(Duration::from_secs(10)).expect("started");
            assert_eq!((part, total), (dir.path().join("t.part"), body.len() as u64));
            let mut reader = arrived.reader(std::sync::Arc::new(AtomicBool::new(false))).unwrap();
            let mut first = vec![0u8; 256 * 1024];
            reader.read_exact(&mut first).unwrap();
            assert_eq!(first, body[..first.len()]);
            assert_eq!(arrived.outcome(), None, "read before the download ended");
            assert_eq!(download.join().unwrap().unwrap(), body.len() as u64);
            reader
        });
        let mut reader = reader;
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, body[256 * 1024..], "the reader keeps the renamed file");
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(arrived.outcome(), Some(crate::arrived::Outcome::Complete));
    }
}
