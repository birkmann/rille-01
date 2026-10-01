//! Track suggestions: collection tracks that mix well after a reference
//! track (the one playing), scored by tempo, key and genre.

use std::collections::HashSet;

use rille_core::Key;
use rille_library::{TrackId, TrackRow};

/// How much tempo, key and genre count (of 1). A part the reference track
/// has no data for is left out and the others share its weight.
const TEMPO_WEIGHT: f32 = 0.4;
const KEY_WEIGHT: f32 = 0.35;
const STYLE_WEIGHT: f32 = 0.25;
/// Tempo difference at which the tempo fit reaches 0 (6 %).
const TEMPO_SPAN: f64 = 0.06;
/// Half/double-time matches count this much of a straight match.
const HALF_DOUBLE: f64 = 0.8;
/// Tracks scoring lower are not suggested.
pub const MIN_SCORE: f32 = 0.5;
pub const MAX_SUGGESTIONS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Suggestion {
    pub id: TrackId,
    /// 0..=1.
    pub score: f32,
}

/// 1 at the same tempo, falling to 0 at [`TEMPO_SPAN`] apart. Half and
/// double tempo fit too (a 87 BPM track after a 174 BPM one), a bit less.
pub fn tempo_fit(reference: f64, candidate: f64) -> f32 {
    if reference <= 0.0 || candidate <= 0.0 {
        return 0.0;
    }
    let fit =
        |factor: f64, weight: f64| weight * (1.0 - (candidate * factor / reference - 1.0).abs() / TEMPO_SPAN).max(0.0);
    fit(1.0, 1.0).max(fit(2.0, HALF_DOUBLE)).max(fit(0.5, HALF_DOUBLE)) as f32
}

/// Harmonic fit on the Camelot wheel: same key, one step around the wheel,
/// the relative major/minor, then two steps (energy boost) and the
/// diagonal neighbours; everything else does not fit.
pub fn key_fit(a: Key, b: Key) -> f32 {
    let d = (i16::from(a.camelot_number()) - i16::from(b.camelot_number())).rem_euclid(12);
    let steps = d.min(12 - d);
    match (steps, a.is_minor() == b.is_minor()) {
        (0, true) => 1.0,
        (1, true) => 0.85,
        (0, false) => 0.8,
        (2, true) | (1, false) => 0.4,
        _ => 0.0,
    }
}

/// Lowercase words of a genre tag, so "Drum & Bass", "drum and bass" and
/// "Drum-n-Bass" compare equal.
fn genre_words(genre: &str) -> Vec<String> {
    let mut w: Vec<String> = genre
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| !w.is_empty() && !matches!(w.as_str(), "and" | "n"))
        .collect();
    w.sort();
    w.dedup();
    w
}

/// 1 for the same genre, up to 0.7 for genres sharing words ("Techno" and
/// "Melodic House & Techno"), 0 otherwise or when untagged.
fn style_fit(reference: &[String], candidate: &[String]) -> f32 {
    if candidate.is_empty() {
        return 0.0;
    }
    if reference == candidate {
        return 1.0;
    }
    let common = candidate.iter().filter(|w| reference.contains(w)).count();
    0.7 * common as f32 / reference.len().min(candidate.len()) as f32
}

/// How well `candidate` mixes after `reference`, 0..=1, or `None` when the
/// reference has a tempo and the candidate's tempo does not fit it (or is
/// unknown): those cannot be mixed in tempo.
fn score(reference: &TrackRow, ref_genre: &[String], candidate: &TrackRow) -> Option<f32> {
    let (mut total, mut weight) = (0.0, 0.0);
    if let Some(bpm) = reference.bpm {
        let fit = candidate.bpm.map_or(0.0, |b| tempo_fit(bpm, b));
        if fit <= 0.0 {
            return None;
        }
        total += TEMPO_WEIGHT * fit;
        weight += TEMPO_WEIGHT;
    }
    if let Some(key) = reference.key {
        total += KEY_WEIGHT * candidate.key.map_or(0.0, |k| key_fit(key, k));
        weight += KEY_WEIGHT;
    }
    if !ref_genre.is_empty() {
        total += STYLE_WEIGHT * style_fit(ref_genre, &genre_words(&candidate.genre));
        weight += STYLE_WEIGHT;
    }
    (weight > 0.0).then(|| total / weight)
}

/// The best tracks of `rows` to play after `reference`, best first (equal
/// scores: higher rated first). Leaves out `exclude` (tracks on the decks,
/// already played), missing files and other copies of the reference.
pub fn suggest(reference: &TrackRow, rows: &[TrackRow], exclude: &HashSet<TrackId>) -> Vec<Suggestion> {
    let ref_genre = genre_words(&reference.genre);
    let same_song = |r: &TrackRow| {
        !reference.title.is_empty()
            && r.title.eq_ignore_ascii_case(&reference.title)
            && r.artist.eq_ignore_ascii_case(&reference.artist)
    };
    let mut scored: Vec<(f32, &TrackRow)> = rows
        .iter()
        .filter(|r| r.id != reference.id && !r.missing && !exclude.contains(&r.id) && !same_song(r))
        .filter_map(|r| score(reference, &ref_genre, r).filter(|s| *s >= MIN_SCORE).map(|s| (s, r)))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| b.1.rating.cmp(&a.1.rating)));
    scored.truncate(MAX_SUGGESTIONS);
    scored.into_iter().map(|(score, r)| Suggestion { id: r.id, score }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn track(id: TrackId, bpm: Option<f64>, key: Option<Key>, genre: &str) -> TrackRow {
        TrackRow {
            id,
            path: PathBuf::from(format!("/music/{id}.mp3")),
            title: format!("Track {id}"),
            artist: "Artist".into(),
            album: String::new(),
            remixer: String::new(),
            label: String::new(),
            genre: genre.into(),
            comment: String::new(),
            year: None,
            duration_secs: 300.0,
            bpm,
            key,
            rating: 0,
            color: None,
            play_count: 0,
            last_played: None,
            date_added: 0,
            file_size: 0,
            bitrate: None,
            sample_rate: None,
            has_cover: false,
            cover: None,
            grid_confidence: None,
            grid_flags: 0,
            grid_locked: false,
            analyzed: true,
            analysis_version: None,
            analysis_failed: false,
            missing: false,
        }
    }

    /// 8A (A minor).
    fn am() -> Key {
        Key::new(9, true)
    }

    #[test]
    fn tempo() {
        assert_eq!(tempo_fit(128.0, 128.0), 1.0);
        assert!(tempo_fit(128.0, 126.0) > 0.7);
        assert_eq!(tempo_fit(128.0, 140.0), 0.0);
        assert!((tempo_fit(174.0, 87.0) - 0.8).abs() < 1e-6, "half time");
        assert!((tempo_fit(87.0, 174.0) - 0.8).abs() < 1e-6, "double time");
        assert_eq!(tempo_fit(0.0, 128.0), 0.0);
    }

    #[test]
    fn keys_on_the_camelot_wheel() {
        let k = |cam: u8, minor: bool| {
            (0..24u8)
                .map(|v| Key::try_from(v).unwrap())
                .find(|k| k.camelot_number() == cam && k.is_minor() == minor)
                .unwrap()
        };
        assert_eq!(key_fit(am(), k(8, true)), 1.0);
        assert_eq!(key_fit(am(), k(9, true)), 0.85);
        assert_eq!(key_fit(am(), k(7, true)), 0.85);
        assert_eq!(key_fit(am(), k(8, false)), 0.8, "relative major");
        assert_eq!(key_fit(am(), k(10, true)), 0.4, "energy boost");
        assert_eq!(key_fit(am(), k(9, false)), 0.4, "diagonal");
        assert_eq!(key_fit(k(12, true), k(1, true)), 0.85, "wraps around");
        assert_eq!(key_fit(am(), k(2, true)), 0.0);
    }

    #[test]
    fn genres() {
        let g = |s: &str| genre_words(s);
        assert_eq!(g("Drum & Bass"), g("drum and bass"));
        assert_eq!(g("Drum-n-Bass"), g("Drum & Bass"));
        assert_eq!(style_fit(&g("Techno"), &g("techno")), 1.0);
        assert!((style_fit(&g("Techno"), &g("Melodic House & Techno")) - 0.7).abs() < 1e-6);
        assert!((style_fit(&g("Tech House"), &g("Deep House")) - 0.35).abs() < 1e-6);
        assert_eq!(style_fit(&g("Techno"), &g("Trance")), 0.0);
        assert_eq!(style_fit(&g("Techno"), &g("")), 0.0);
    }

    #[test]
    fn ranks_by_tempo_key_and_genre() {
        let reference = track(1, Some(128.0), Some(am()), "Techno");
        let rows = [
            reference.clone(),
            track(2, Some(128.0), Some(am()), "Techno"), // perfect
            track(3, Some(127.0), Some(am().transposed(7)), "Techno"), // one step (Em, 9A)
            track(4, Some(128.0), Some(am()), "Trance"), // other genre
            track(5, Some(150.0), Some(am()), "Techno"), // tempo too far
            track(6, None, Some(am()), "Techno"),        // not analyzed
            track(7, Some(64.0), Some(am()), "Techno"),  // half time
            track(8, Some(128.0), Some(Key::new(1, false)), "Trance"), // nothing but tempo
            track(9, Some(128.0), Some(am()), "Techno"), // on a deck
        ];
        let exclude: HashSet<TrackId> = [9].into();
        let s = suggest(&reference, &rows, &exclude);
        let ids: Vec<TrackId> = s.iter().map(|s| s.id).collect();
        assert_eq!(ids, [2, 7, 3, 4], "8 only fits the tempo");
        assert_eq!(s[0].score, 1.0);
        assert!(s.windows(2).all(|w| w[0].score >= w[1].score));
    }

    #[test]
    fn missing_reference_data_is_left_out() {
        // No key and no genre: tempo alone decides.
        let reference = track(1, Some(128.0), None, "");
        let rows = [track(2, Some(128.0), None, ""), track(3, Some(126.0), Some(am()), "House")];
        let s = suggest(&reference, &rows, &HashSet::new());
        assert_eq!(s[0], Suggestion { id: 2, score: 1.0 });
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn copies_of_the_reference_and_missing_files_are_skipped() {
        let reference = track(1, Some(128.0), Some(am()), "Techno");
        let mut copy = track(2, Some(128.0), Some(am()), "Techno");
        copy.title = "TRACK 1".into();
        let mut gone = track(3, Some(128.0), Some(am()), "Techno");
        gone.missing = true;
        assert!(suggest(&reference, &[copy, gone], &HashSet::new()).is_empty());
    }
}
