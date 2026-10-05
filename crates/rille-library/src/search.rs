//! In-memory search over collection rows.

use crate::TrackRow;

/// Case-insensitive substring search. Every whitespace-separated query term
/// must occur in one of title, artist, album, remixer, label, genre, comment
/// or file name. Indices refer to the row slice the index was built from.
#[derive(Clone, Debug, Default)]
pub struct SearchIndex {
    haystacks: Vec<Box<str>>,
}

impl SearchIndex {
    pub fn new(rows: &[TrackRow]) -> Self {
        Self { haystacks: rows.iter().map(haystack).collect() }
    }

    pub fn len(&self) -> usize {
        self.haystacks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.haystacks.is_empty()
    }

    /// Replaces row `i` after an edit, or appends when `i == len()`.
    pub fn set(&mut self, i: usize, row: &TrackRow) {
        match self.haystacks.get_mut(i) {
            Some(h) => *h = haystack(row),
            None => self.haystacks.push(haystack(row)),
        }
    }

    /// Matching row indices in ascending order; an empty query matches all.
    pub fn search(&self, query: &str) -> Vec<usize> {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let hits = self.haystacks.iter().enumerate();
        hits.filter(|(_, h)| terms.iter().all(|t| h.contains(t.as_str()))).map(|(i, _)| i).collect()
    }
}

/// Fields joined by '\n', which no term can contain, so terms never match
/// across a field boundary.
fn haystack(r: &TrackRow) -> Box<str> {
    let file = r.path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default();
    let fields = [&r.title, &r.artist, &r.album, &r.remixer, &r.label, &r.genre, &r.comment];
    let mut s = String::with_capacity(fields.iter().map(|f| f.len() + 1).sum::<usize>() + file.len());
    for f in fields {
        s.push_str(f);
        s.push('\n');
    }
    s.push_str(&file);
    s.to_lowercase().into_boxed_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn row(title: &str, artist: &str, file: &str) -> TrackRow {
        TrackRow {
            id: 0,
            path: PathBuf::from("/music").join(file),
            title: title.into(),
            artist: artist.into(),
            album: String::new(),
            remixer: String::new(),
            label: "Drumcode".into(),
            genre: "Techno".into(),
            comment: String::new(),
            year: None,
            duration_secs: 0.0,
            bpm: None,
            key: None,
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
            analysis_version: None,
            analysis_failed: false,
            grid_confidence: None,
            grid_flags: 0,
            grid_locked: false,
            analyzed: false,
            missing: false,
            beatport_id: None,
            beatport_offline: false,
            guest: false,
        }
    }

    #[test]
    fn semantics() {
        let rows = [
            row("Polymorph", "Easy Peelers", "a.mp3"),
            row("The Looker", "Matt Meler", "b.flac"),
            row("Ünïcode Wörld", "Björk", "c_special.wav"),
        ];
        let idx = SearchIndex::new(&rows);
        assert_eq!(idx.search(""), [0, 1, 2]);
        assert_eq!(idx.search("   "), [0, 1, 2]);
        assert_eq!(idx.search("POLY"), [0]);
        assert_eq!(idx.search("matt looker"), [1]);
        assert_eq!(idx.search("matt polymorph"), Vec::<usize>::new());
        assert_eq!(idx.search("techno"), [0, 1, 2]);
        assert_eq!(idx.search("flac"), [1]);
        assert_eq!(idx.search("special"), [2]);
        assert_eq!(idx.search("BJÖRK wörld"), [2]);
        // No match across field boundaries.
        assert_eq!(idx.search("peelersdrumcode"), Vec::<usize>::new());
        assert_eq!(idx.search("music"), Vec::<usize>::new(), "directories are not searched");

        let mut idx = idx;
        idx.set(1, &row("Changed", "Someone", "b.flac"));
        assert_eq!(idx.search("looker"), Vec::<usize>::new());
        assert_eq!(idx.search("changed"), [1]);
    }

    #[test]
    fn fifty_thousand_rows() {
        let rows: Vec<TrackRow> = (0..50_000)
            .map(|i| row(&format!("Track Title Number {i}"), &format!("Artist {}", i % 997), &format!("{i:05}.mp3")))
            .collect();
        let idx = SearchIndex::new(&rows);
        let t = std::time::Instant::now();
        let hits = idx.search("artist 42 number");
        let elapsed = t.elapsed();
        assert!(hits.len() > 50);
        // Release target is < 50 ms; debug builds are allowed a lot more.
        let limit = if cfg!(debug_assertions) { 2000 } else { 50 };
        assert!(elapsed.as_millis() < limit, "{elapsed:?}");
    }
}
