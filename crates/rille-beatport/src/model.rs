//! The parts of Beatport's catalog JSON that rille uses. Every field is
//! optional on the wire: missing or `null` values become their default.

use serde::{Deserialize, Deserializer};

fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Named {
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Image {
    #[serde(deserialize_with = "nullable")]
    pub uri: String,
    /// Has a `{w}x{h}` placeholder for the size.
    #[serde(deserialize_with = "nullable")]
    pub dynamic_uri: String,
}

impl Image {
    /// The image scaled to `px`×`px`, if there is one.
    pub fn url(&self, px: u32) -> Option<String> {
        if !self.dynamic_uri.is_empty() {
            Some(self.dynamic_uri.replace("{w}x{h}", &format!("{px}x{px}")))
        } else {
            Some(self.uri.clone()).filter(|u| !u.is_empty())
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TrackKey {
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    pub camelot_number: Option<u8>,
    #[serde(deserialize_with = "nullable")]
    pub camelot_letter: String,
}

impl TrackKey {
    /// Camelot notation, e.g. "8A".
    pub fn camelot(&self) -> Option<String> {
        let n = self.camelot_number.filter(|n| (1..=12).contains(n))?;
        let l = self.camelot_letter.trim().to_ascii_uppercase();
        matches!(l.as_str(), "A" | "B").then(|| format!("{n}{l}"))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Release {
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    pub label: Option<Named>,
    pub image: Option<Image>,
    pub new_release_date: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Track {
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(deserialize_with = "nullable")]
    pub mix_name: String,
    #[serde(deserialize_with = "nullable")]
    pub slug: String,
    #[serde(deserialize_with = "nullable")]
    pub artists: Vec<Named>,
    #[serde(deserialize_with = "nullable")]
    pub remixers: Vec<Named>,
    pub bpm: Option<f64>,
    pub key: Option<TrackKey>,
    pub genre: Option<Named>,
    pub sub_genre: Option<Named>,
    pub length_ms: Option<u64>,
    pub publish_date: Option<String>,
    pub release: Option<Release>,
    /// `false` for tracks only sold, not streamed (seen in purchases).
    pub is_available_for_streaming: Option<bool>,
}

impl Track {
    /// "Name (Mix)", as Beatport shows it.
    pub fn display_title(&self) -> String {
        if self.mix_name.trim().is_empty() { self.name.clone() } else { format!("{} ({})", self.name, self.mix_name) }
    }

    pub fn artist_names(&self) -> String {
        join_names(&self.artists)
    }

    pub fn remixer_names(&self) -> String {
        join_names(&self.remixers)
    }

    pub fn label_name(&self) -> String {
        self.release.as_ref().and_then(|r| r.label.as_ref()).map_or_else(String::new, |l| l.name.clone())
    }

    pub fn genre_name(&self) -> String {
        self.genre.as_ref().map_or_else(String::new, |g| g.name.clone())
    }

    pub fn year(&self) -> Option<i32> {
        let date = self.publish_date.as_deref().or(self.release.as_ref()?.new_release_date.as_deref())?;
        date.get(..4)?.parse().ok()
    }

    pub fn duration_secs(&self) -> f64 {
        self.length_ms.map_or(0.0, |ms| ms as f64 / 1000.0)
    }

    pub fn cover_url(&self, px: u32) -> Option<String> {
        self.release.as_ref()?.image.as_ref()?.url(px)
    }

    /// The track's page on beatport.com.
    pub fn web_url(&self) -> String {
        let slug = if self.slug.is_empty() { "track" } else { &self.slug };
        format!("https://www.beatport.com/track/{slug}/{}", self.id)
    }
}

fn join_names(n: &[Named]) -> String {
    n.iter().map(|a| a.name.as_str()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")
}

/// A page of a list endpoint.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, bound(deserialize = "T: Deserialize<'de>"))]
pub struct Paginated<T> {
    pub next: Option<String>,
    pub count: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    pub results: Vec<T>,
}

impl<T> Default for Paginated<T> {
    fn default() -> Self {
        Self { next: None, count: None, results: Vec::new() }
    }
}

/// An entry of a playlist: the track plus its position.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct PlaylistItem {
    pub position: Option<u32>,
    pub track: Option<Track>,
}

/// A playlist in the user's Beatport library.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Playlist {
    pub id: i64,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    pub track_count: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct SearchResults {
    #[serde(deserialize_with = "nullable")]
    pub tracks: Vec<Track>,
}

/// Where to fetch a track's audio file.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Download {
    #[serde(deserialize_with = "nullable")]
    pub location: String,
    /// ".flac", ".256k.aac.mp4" or ".128k.aac.mp4".
    #[serde(deserialize_with = "nullable")]
    pub stream_quality: String,
}

impl Download {
    /// File extension for the downloaded audio.
    pub fn extension(&self) -> &'static str {
        if self.stream_quality.contains("flac") { "flac" } else { "m4a" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: &str = r#"{
        "id": 17654321, "name": "Polymorph", "mix_name": "Original Mix", "slug": "polymorph",
        "artists": [{"id": 1, "name": "Easy Peelers"}, {"id": 2, "name": "Someone"}],
        "remixers": [], "bpm": 128,
        "key": {"name": "A Minor", "camelot_number": 8, "camelot_letter": "A"},
        "genre": {"id": 6, "name": "Techno (Peak Time / Driving)"}, "sub_genre": null,
        "length_ms": 412345, "publish_date": "2023-04-07",
        "release": {"id": 9, "name": "Polymorph EP", "label": {"id": 3, "name": "Drumcode"},
                    "image": {"uri": "https://geo-media.beatport.com/image_size/1400x1400/abc.jpg",
                              "dynamic_uri": "https://geo-media.beatport.com/image_size/{w}x{h}/abc.jpg"}},
        "isrc": "X", "unknown_field": {"a": 1}
    }"#;

    #[test]
    fn track_fields() {
        let t: Track = serde_json::from_str(TRACK).unwrap();
        assert_eq!(t.display_title(), "Polymorph (Original Mix)");
        assert_eq!(t.artist_names(), "Easy Peelers, Someone");
        assert_eq!(t.remixer_names(), "");
        assert_eq!(t.bpm, Some(128.0));
        assert_eq!(t.key.as_ref().and_then(TrackKey::camelot).as_deref(), Some("8A"));
        assert_eq!(t.label_name(), "Drumcode");
        assert_eq!(t.genre_name(), "Techno (Peak Time / Driving)");
        assert_eq!(t.year(), Some(2023));
        assert!((t.duration_secs() - 412.345).abs() < 1e-9);
        assert_eq!(t.cover_url(500).as_deref(), Some("https://geo-media.beatport.com/image_size/500x500/abc.jpg"));
        assert_eq!(t.web_url(), "https://www.beatport.com/track/polymorph/17654321");
    }

    #[test]
    fn nulls_and_missing_fields() {
        let t: Track = serde_json::from_str(r#"{"id": 5, "name": null, "mix_name": null, "artists": null}"#).unwrap();
        assert_eq!((t.id, t.display_title(), t.artist_names()), (5, String::new(), String::new()));
        assert_eq!(t.cover_url(500), None);
        assert_eq!(t.key.as_ref().and_then(TrackKey::camelot), None);
        let k = TrackKey { camelot_number: Some(13), camelot_letter: "A".into(), ..TrackKey::default() };
        assert_eq!(k.camelot(), None);
    }

    #[test]
    fn pages_search_and_playlists() {
        let page: Paginated<PlaylistItem> = serde_json::from_str(&format!(
            r#"{{"next": "https://api.beatport.com/v4/x/?page=2", "count": 3, "page": "1/2", "per_page": 1,
                "results": [{{"id": 1, "position": 1, "track": {TRACK}}}]}}"#
        ))
        .unwrap();
        assert!(page.next.is_some());
        assert_eq!(page.results[0].track.as_ref().map(|t| t.id), Some(17654321));
        let s: SearchResults = serde_json::from_str(&format!(r#"{{"tracks": [{TRACK}], "releases": []}}"#)).unwrap();
        assert_eq!(s.tracks.len(), 1);
        let p: Paginated<Playlist> =
            serde_json::from_str(r#"{"next": null, "results": [{"id": 7, "name": "Warmup", "track_count": 12}]}"#)
                .unwrap();
        assert_eq!(p.results, vec![Playlist { id: 7, name: "Warmup".into(), track_count: Some(12) }]);
        let d: Download =
            serde_json::from_str(r#"{"location": "https://x/y.flac", "stream_quality": ".flac"}"#).unwrap();
        assert_eq!(d.extension(), "flac");
        let d: Download = serde_json::from_str(r#"{"location": "u", "stream_quality": ".256k.aac.mp4"}"#).unwrap();
        assert_eq!(d.extension(), "m4a");
    }
}
