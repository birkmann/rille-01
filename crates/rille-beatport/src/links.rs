//! Beatport web and API links: `https://www.beatport.com/track/slug/123`,
//! `/library/playlists/456`, `https://api.beatport.com/v4/catalog/releases/789/`.

/// What a link points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Track(i64),
    Release(i64),
    /// A user's library playlist.
    Playlist(i64),
    /// A DJ chart (shown as "playlist" on beatport.com).
    Chart(i64),
    Label(i64),
    Artist(i64),
    Genre(i64),
}

impl Link {
    pub fn kind(self) -> &'static str {
        match self {
            Self::Track(_) => "track",
            Self::Release(_) => "release",
            Self::Playlist(_) => "playlist",
            Self::Chart(_) => "chart",
            Self::Label(_) => "label",
            Self::Artist(_) => "artist",
            Self::Genre(_) => "genre",
        }
    }
}

/// Whether `text` looks like a Beatport link at all (worth parsing instead
/// of searching for).
pub fn looks_like_link(text: &str) -> bool {
    let t = text.trim();
    let rest = t.strip_prefix("https://").or_else(|| t.strip_prefix("http://")).unwrap_or(t);
    ["www.beatport.com/", "beatport.com/", "api.beatport.com/"].iter().any(|h| rest.starts_with(h))
}

/// Parses a Beatport link (the same forms beatportdl accepts).
pub fn parse(url: &str) -> Result<Link, String> {
    let t = url.trim();
    let rest = t.strip_prefix("https://").or_else(|| t.strip_prefix("http://")).unwrap_or(t);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    if !matches!(host, "www.beatport.com" | "beatport.com" | "api.beatport.com") {
        return Err("not a Beatport link".into());
    }
    let path = path.split(['?', '#']).next().unwrap_or("");
    let mut seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // Language ("/de/track/…") or API version ("/v4/catalog/tracks/…").
    if seg.len() > 1 && seg[0].len() == 2 {
        seg.remove(0);
        if seg.first() == Some(&"catalog") {
            seg.remove(0);
        }
    }
    let Some(&first) = seg.first() else { return Err("not a Beatport link".into()) };
    let (make, at): (fn(i64) -> Link, usize) = match first {
        "track" => (Link::Track, 2),
        "release" => (Link::Release, 2),
        "library" if matches!(seg.get(1), Some(&("playlists" | "playlist"))) => (Link::Playlist, 2),
        "playlists" => (Link::Playlist, 2),
        "chart" | "playlist" => (Link::Chart, 2),
        "label" => (Link::Label, 2),
        "artist" => (Link::Artist, 2),
        "genre" => (Link::Genre, 2),
        "tracks" => (Link::Track, 1),
        "releases" => (Link::Release, 1),
        "artists" => (Link::Artist, 1),
        "labels" => (Link::Label, 1),
        "charts" => (Link::Chart, 1),
        _ => return Err(format!("unsupported Beatport link: /{first}")),
    };
    let id = seg.get(at).ok_or("Beatport link without an id")?;
    id.parse().map(make).map_err(|_| format!("invalid id in Beatport link: {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_and_api_links() {
        let cases = [
            ("https://www.beatport.com/track/polymorph/17654321", Link::Track(17654321)),
            ("https://www.beatport.com/de/track/polymorph/17654321?foo=1", Link::Track(17654321)),
            ("www.beatport.com/release/some-release/4567890", Link::Release(4567890)),
            ("https://www.beatport.com/library/playlists/123", Link::Playlist(123)),
            ("https://www.beatport.com/playlists/share/77", Link::Playlist(77)),
            ("https://www.beatport.com/chart/my-chart/890", Link::Chart(890)),
            ("https://www.beatport.com/playlist/my-chart/891", Link::Chart(891)),
            ("https://www.beatport.com/label/drumcode/1", Link::Label(1)),
            ("https://www.beatport.com/genre/techno-peak-time/6/top-100", Link::Genre(6)),
            ("https://api.beatport.com/v4/catalog/tracks/42/", Link::Track(42)),
            ("https://api.beatport.com/v4/catalog/releases/43", Link::Release(43)),
        ];
        for (url, want) in cases {
            assert_eq!(parse(url), Ok(want), "{url}");
            assert!(looks_like_link(url), "{url}");
        }
    }

    #[test]
    fn rejects_other_links() {
        for bad in [
            "https://www.beatsource.com/track/x/1",
            "https://example.com/track/x/1",
            "https://www.beatport.com/",
            "https://www.beatport.com/track/slug",
            "https://www.beatport.com/track/slug/abc",
            "https://www.beatport.com/search?q=x",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(!looks_like_link("solomun"));
    }
}
