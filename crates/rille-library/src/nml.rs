//! Traktor `collection.nml` import: grids, cues, ratings and a few tag
//! fields, matched to files through a caller-supplied path mapping.

use crate::library::write_grid;
use crate::{Library, Result, TrackId, io_err};
use quick_xml::encoding::Decoder;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use rille_core::beatgrid::{PiecewiseMap, TempoMarker};
use rille_core::track::HOTCUE_COLORS;
use rille_core::{BeatGrid, BeatMap, CueKind, CuePoint, GridFlags, GridSource, TrackCues};
use rusqlite::{OptionalExtension, params};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct NmlReport {
    /// `<ENTRY>` elements in the collection.
    pub entries: usize,
    /// Entries resolved to a library track (including newly imported ones).
    pub matched: usize,
    /// Tracks added to the library by this import.
    pub imported: usize,
    pub grids: usize,
    /// Cue points (all kinds) imported.
    pub cues: usize,
    /// NML paths that did not map to an existing file.
    pub missing: Vec<String>,
    /// Files that exist but could not be imported, with the reason.
    pub errors: Vec<(PathBuf, String)>,
}

#[derive(Debug, Default)]
struct Entry {
    dir: String,
    file: String,
    lock: bool,
    rating: Option<u8>,
    comment: Option<String>,
    genre: Option<String>,
    label: Option<String>,
    color: Option<u32>,
    play_count: Option<u32>,
    bpm: Option<f64>,
    key: Option<u8>,
    cues: Vec<Cue>,
}

#[derive(Debug, Default)]
struct Cue {
    name: String,
    kind: i32,
    start_ms: f64,
    len_ms: f64,
    hotcue: i32,
    color: Option<u32>,
    /// `<GRID BPM>` child of a grid marker.
    grid_bpm: Option<f64>,
}

/// Traktor's track colors 1..=7.
const TRACK_COLORS: [u32; 7] = [0xe8453c, 0xf5a524, 0xf2c94c, 0x5fd068, 0x4f8fe8, 0x9b6bd6, 0xd96bd6];

impl Library {
    /// Imports a Traktor collection. `path_map` turns an NML path such as
    /// "/Users/me/Music/a.mp3" (volume dropped) into a local path, or `None`
    /// to skip it; pass `&|p| Some(p.into())` to use paths as they are.
    ///
    /// Grids replace analyzed or previously imported ones but never a grid
    /// the user edited or locked here (a lock imported from Traktor does not
    /// block the next NML import). Cues replace existing cues when the
    /// entry has any.
    pub fn import_nml(&mut self, nml_path: &Path, path_map: &dyn Fn(&str) -> Option<PathBuf>) -> Result<NmlReport> {
        let entries = parse(nml_path)?;
        let mut report = NmlReport { entries: entries.len(), ..NmlReport::default() };
        for e in entries {
            let nml = nml_path_string(&e.dir, &e.file);
            let Some(path) = path_map(&nml).filter(|p| p.is_file()) else {
                report.missing.push(nml);
                continue;
            };
            let id = match self.track_by_path(&path)? {
                Some(id) => id,
                None => match self.import_file(&path) {
                    Ok(id) => {
                        report.imported += 1;
                        id
                    }
                    Err(err) => {
                        report.errors.push((path, err.to_string()));
                        continue;
                    }
                },
            };
            report.matched += 1;
            self.apply_entry(id, &e, &mut report)?;
        }
        Ok(report)
    }

    fn apply_entry(&mut self, id: TrackId, e: &Entry, report: &mut NmlReport) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE tracks SET rating = COALESCE(?2, rating), comment = COALESCE(?3, comment),
             genre = COALESCE(?4, genre), label = COALESCE(?5, label), color = COALESCE(?6, color),
             play_count = MAX(play_count, COALESCE(?7, 0)), musical_key = COALESCE(?8, musical_key),
             bpm = COALESCE(?9, bpm) WHERE id = ?1",
            params![id, e.rating, e.comment, e.genre, e.label, e.color, e.play_count, e.key, e.bpm],
        )?;
        if let Some(mut grid) = build_grid(e) {
            grid.locked = e.lock;
            let user_owned = tx
                .query_row(
                    "SELECT user_edited OR (locked AND source <> 'imported_nml') FROM beatgrids WHERE track_id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(false);
            if !user_owned {
                write_grid(&tx, id, &grid, false)?;
                report.grids += 1;
            }
        }
        if let Some(cues) = build_cues(&e.cues) {
            report.cues += cues.points.len();
            tx.execute("UPDATE tracks SET cues = ?2 WHERE id = ?1", params![id, serde_json::to_string(&cues)?])?;
        }
        Ok(tx.commit()?)
    }
}

/// "/:Users/:me/:Music/:" + "a.mp3" -> "/Users/me/Music/a.mp3"
fn nml_path_string(dir: &str, file: &str) -> String {
    let mut p = dir.replace("/:", "/");
    if !p.starts_with('/') {
        p.insert(0, '/');
    }
    if !p.ends_with('/') {
        p.push('/');
    }
    p.push_str(file);
    while p.contains("//") {
        p = p.replace("//", "/");
    }
    p
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn parse(path: &Path) -> Result<Vec<Entry>> {
    let file = std::fs::File::open(path).map_err(io_err(path))?;
    let mut reader = Reader::from_reader(std::io::BufReader::new(file));
    let mut buf = Vec::new();
    let mut entries = Vec::new();
    let (mut in_collection, mut entry, mut cue) = (false, None::<Entry>, None::<Cue>);
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        let d = reader.decoder();
        match &ev {
            Event::Start(e) | Event::Empty(e) => {
                let empty = matches!(ev, Event::Empty(_));
                match (e.name().as_ref(), entry.as_mut()) {
                    (b"COLLECTION", _) => in_collection = !empty,
                    (b"ENTRY", None) if in_collection => {
                        let new = Entry { lock: attr(e, d, b"LOCK").as_deref() == Some("1"), ..Entry::default() };
                        if empty {
                            entries.push(new);
                        } else {
                            entry = Some(new);
                        }
                    }
                    (b"LOCATION", Some(en)) => {
                        en.dir = attr(e, d, b"DIR").unwrap_or_default();
                        en.file = attr(e, d, b"FILE").unwrap_or_default();
                    }
                    (b"INFO", Some(en)) => {
                        en.rating = num::<u32>(e, d, b"RANKING").map(|r| (r.min(255) / 51) as u8);
                        en.comment = attr(e, d, b"COMMENT").filter(|s| !s.is_empty());
                        en.genre = attr(e, d, b"GENRE").filter(|s| !s.is_empty());
                        en.label = attr(e, d, b"LABEL").filter(|s| !s.is_empty());
                        en.play_count = num(e, d, b"PLAYCOUNT");
                        en.color =
                            num::<usize>(e, d, b"COLOR").and_then(|c| TRACK_COLORS.get(c.wrapping_sub(1)).copied());
                    }
                    (b"TEMPO", Some(en)) => en.bpm = num::<f64>(e, d, b"BPM").filter(|b| *b > 0.0 && b.is_finite()),
                    (b"MUSICAL_KEY", Some(en)) => en.key = num::<u8>(e, d, b"VALUE").filter(|k| *k < 24),
                    (b"CUE_V2", Some(en)) => {
                        let c = Cue {
                            name: attr(e, d, b"NAME").unwrap_or_default(),
                            kind: num(e, d, b"TYPE").unwrap_or(0),
                            start_ms: num(e, d, b"START").unwrap_or(0.0),
                            len_ms: num(e, d, b"LEN").unwrap_or(0.0),
                            hotcue: num(e, d, b"HOTCUE").unwrap_or(-1),
                            color: attr(e, d, b"COLOR").and_then(|c| parse_color(&c)),
                            grid_bpm: None,
                        };
                        if empty {
                            en.cues.push(c);
                        } else {
                            cue = Some(c);
                        }
                    }
                    (b"GRID", Some(_)) => {
                        if let Some(c) = cue.as_mut() {
                            c.grid_bpm = num::<f64>(e, d, b"BPM").filter(|b| *b > 0.0 && b.is_finite());
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) => match e.name().as_ref() {
                b"COLLECTION" => in_collection = false,
                b"ENTRY" => entries.extend(entry.take()),
                b"CUE_V2" => {
                    if let (Some(en), Some(c)) = (entry.as_mut(), cue.take()) {
                        en.cues.push(c);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(entries)
}

fn attr(e: &BytesStart, d: Decoder, name: &[u8]) -> Option<String> {
    let a = e.attributes().flatten().find(|a| a.key.as_ref() == name)?;
    a.decoded_and_normalized_value(XmlVersion::Implicit1_0, d).ok().map(|v| v.trim().to_owned())
}

fn num<T: std::str::FromStr>(e: &BytesStart, d: Decoder, name: &[u8]) -> Option<T> {
    attr(e, d, name)?.parse().ok()
}

/// "#RRGGBB", "0xRRGGBB" or a decimal number.
fn parse_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#').or_else(|| s.strip_prefix("0x"));
    let v = match hex {
        Some(h) => u32::from_str_radix(h, 16).ok()?,
        None => s.parse().ok()?,
    };
    Some(v & 0xff_ffff)
}

// ---------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------

/// A constant grid from the first grid marker and the track tempo. Several
/// markers at different positions become a piecewise grid; each segment's
/// tempo is fitted so the next marker lands exactly on a beat, like Traktor's
/// phase reset at every marker.
fn build_grid(e: &Entry) -> Option<BeatGrid> {
    let mut marks: Vec<(f64, f64)> = e
        .cues
        .iter()
        .filter(|c| c.kind == 4)
        .filter_map(|c| Some((c.start_ms / 1000.0, c.grid_bpm.or(e.bpm)?)))
        .filter(|(t, _)| t.is_finite())
        .collect();
    marks.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Drop markers less than half a beat after the previous kept one.
    let mut kept: Vec<(f64, f64)> = Vec::with_capacity(marks.len());
    for m in marks {
        if kept.last().is_none_or(|&(t, bpm)| (m.0 - t) * bpm / 60.0 >= 0.5) {
            kept.push(m);
        }
    }
    let &(anchor, first_bpm) = kept.first()?;
    let map = if kept.len() == 1 {
        BeatMap::constant(anchor, e.bpm.unwrap_or(first_bpm)).ok()?
    } else {
        let markers = (0..kept.len())
            .map(|i| {
                let (t, bpm) = kept[i];
                let bpm = match kept.get(i + 1) {
                    Some(&(next, _)) => {
                        let beats = ((next - t) * bpm / 60.0).round().max(1.0);
                        beats * 60.0 / (next - t)
                    }
                    None => bpm,
                };
                TempoMarker { secs: t, bpm }
            })
            .collect();
        BeatMap::Piecewise(PiecewiseMap::new(0.0, markers).ok()?)
    };
    let mut grid = BeatGrid::new(map, GridSource::ImportedNml);
    if matches!(grid.map, BeatMap::Piecewise(_)) {
        grid.flags |= GridFlags::TEMPO_CHANGE;
    }
    Some(grid)
}

fn build_cues(cues: &[Cue]) -> Option<TrackCues> {
    let points: Vec<CuePoint> = cues
        .iter()
        .filter_map(|c| {
            let kind = match c.kind {
                0 => CueKind::Cue,
                1 => CueKind::FadeIn,
                2 => CueKind::FadeOut,
                3 => CueKind::Load,
                4 => CueKind::Grid,
                5 => CueKind::Loop,
                _ => return None,
            };
            let slot = u8::try_from(c.hotcue).ok().filter(|s| *s < 8);
            Some(CuePoint {
                slot,
                kind,
                start_secs: c.start_ms / 1000.0,
                len_secs: if kind == CueKind::Loop { c.len_ms.max(0.0) / 1000.0 } else { 0.0 },
                // Traktor's placeholder for unnamed cues.
                name: if c.name == "n.n." { String::new() } else { c.name.clone() },
                color: c.color.unwrap_or(HOTCUE_COLORS[usize::from(slot.unwrap_or(0))]),
            })
        })
        .collect();
    if points.is_empty() {
        return None;
    }
    let main_cue_secs = points.iter().find(|p| p.kind == CueKind::Load).map_or(0.0, |p| p.start_secs);
    Some(TrackCues { main_cue_secs, points })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rille_core::BeatClock;

    #[test]
    fn paths() {
        assert_eq!(nml_path_string("/:Users/:me/:Music/:", "a.mp3"), "/Users/me/Music/a.mp3");
        assert_eq!(nml_path_string("/:home/:x/:", "b c.flac"), "/home/x/b c.flac");
        assert_eq!(nml_path_string("", "f.wav"), "/f.wav");
    }

    fn grid_cue(ms: f64, bpm: Option<f64>) -> Cue {
        Cue { kind: 4, start_ms: ms, grid_bpm: bpm, ..Cue::default() }
    }

    #[test]
    fn grids() {
        let mut e = Entry { bpm: Some(128.0), cues: vec![grid_cue(100.0, Some(128.0))], ..Entry::default() };
        let g = build_grid(&e).unwrap();
        assert_eq!(g.map, BeatMap::constant(0.1, 128.0).unwrap());
        assert_eq!(g.source, GridSource::ImportedNml);

        // No grid marker: no grid.
        e.cues.clear();
        assert!(build_grid(&e).is_none());

        // Two markers: tempo of the first segment fitted to land on the second.
        e.cues = vec![grid_cue(60_100.0, Some(130.0)), grid_cue(100.0, Some(128.0))];
        let g = build_grid(&e).unwrap();
        let BeatMap::Piecewise(m) = &g.map else { panic!("{:?}", g.map) };
        assert_eq!(m.markers().len(), 2);
        assert!((m.markers()[0].bpm - 128.0).abs() < 1e-9);
        assert!((g.beat_at(60.1) - 128.0).abs() < 1e-9);
        assert_eq!(m.markers()[1].bpm, 130.0);
        assert!(g.flags.contains(GridFlags::TEMPO_CHANGE));

        // A second marker within half a beat is dropped.
        e.cues = vec![grid_cue(100.0, None), grid_cue(200.0, None)];
        assert!(matches!(build_grid(&e).unwrap().map, BeatMap::Constant(_)));
    }
}
