//! Beatport streaming: signing in, browsing the catalog (search, links,
//! the user's playlists) and downloading tracks to play them. Needs a
//! Beatport streaming subscription; lossless audio needs Professional.
//!
//! Blocking HTTP on the caller's thread, like the rest of rille's
//! background work.

mod auth;
mod client;
pub mod links;
pub mod model;

pub use auth::Token;
pub use client::{Client, Progress, Quality};
pub use links::{Link, looks_like_link, parse as parse_link};
pub use model::{Download, Named, Playlist, Track};

pub(crate) const API: &str = "https://api.beatport.com/v4";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not signed in to Beatport")]
    NotSignedIn,
    #[error("the Beatport session expired: sign in again")]
    SessionExpired,
    #[error("Beatport sign-in failed: {0}")]
    Login(String),
    #[error("Beatport: {detail} (HTTP {status})")]
    Http { status: u16, detail: String },
    #[error("network: {0}")]
    Net(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("unexpected Beatport response: {0}")]
    Decode(String),
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn net(e: ureq::Error) -> Error {
    Error::Net(e.to_string())
}

/// The error of a failed API response: its `detail` or `error` text.
pub(crate) fn api_error(mut res: ureq::http::Response<ureq::Body>) -> Error {
    let status = res.status().as_u16();
    let body = res.body_mut().with_config().limit(64 * 1024).read_to_string().unwrap_or_default();
    let detail = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            ["detail", "error_description", "error", "message"]
                .iter()
                .find_map(|k| v.get(k).and_then(|d| d.as_str()).map(str::to_owned))
        })
        .unwrap_or_else(|| res.status().canonical_reason().unwrap_or("request failed").to_owned());
    Error::Http { status, detail }
}

/// The headers of a desktop browser, which the API expects.
/// Exactly beatportdl's headers, which the API is known to answer with JSON.
pub(crate) fn with_headers<B>(req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
    const ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,\
                          */*;q=0.8,application/signed-exchange;v=b3;q=0.7";
    with_accept(req, ACCEPT)
}

/// Browser headers asking for `accept`.
pub(crate) fn with_accept<B>(req: ureq::RequestBuilder<B>, accept: &str) -> ureq::RequestBuilder<B> {
    const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
                              Chrome/123.0.0.0 Safari/537.36";
    req.header("accept", accept)
        .header("accept-language", "en-US,en;q=0.9")
        .header("cache-control", "max-age=0")
        .header("user-agent", USER_AGENT)
}
