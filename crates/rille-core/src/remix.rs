//! Remix deck layout and colours, shared by engine, controllers and UI.
//!
//! A remix deck has [`SLOTS`] slots (columns) of [`ROWS`] cells each. A cell
//! holds one sample (a loop or a one-shot); each slot plays one cell at a
//! time. Cells are numbered `slot * ROWS + row`. Controllers with a 4×4 pad
//! grid see one page of [`PAGE_ROWS`] rows at a time.

pub const SLOTS: usize = 4;
pub const ROWS: usize = 16;
pub const CELLS: usize = SLOTS * ROWS;
/// Rows on one page of a 4×4 pad grid.
pub const PAGE_ROWS: usize = 4;
pub const PAGES: usize = ROWS / PAGE_ROWS;

/// Cell colours: index 0 is "no colour" (shown white), 1..=16 hues around the
/// wheel. Own palette, `0xRRGGBB`.
pub const COLORS: [u32; 17] = [
    0xd8dce2, // white
    0xe5484d, // red
    0xf0663a, // vermilion
    0xf5a524, // orange
    0xf2c94c, // yellow
    0xc6d83f, // lime
    0x7fd250, // green
    0x3fcf8e, // emerald
    0x2ec4b6, // teal
    0x35b8e0, // sky
    0x4f8ff0, // blue
    0x6b72f2, // indigo
    0x9466f0, // violet
    0xbf5fe8, // purple
    0xe05fc8, // magenta
    0xec5c94, // pink
    0xb08a64, // tan
];

/// Default colour of samples loaded into each slot.
pub const SLOT_COLORS: [u8; SLOTS] = [3, 8, 12, 6];

/// Cell index for pad `pad` (`1..=16`, row by row from the top left) on
/// page `page`; `None` for pads out of range.
pub fn pad_cell(pad: u8, page: usize) -> Option<usize> {
    let p = usize::from(pad).checked_sub(1).filter(|&p| p < SLOTS * PAGE_ROWS)?;
    let (row, slot) = (p / SLOTS, p % SLOTS);
    Some(slot * ROWS + (page.min(PAGES - 1) * PAGE_ROWS + row))
}

/// (slot, row) of a cell.
pub fn cell_slot_row(cell: usize) -> (usize, usize) {
    (cell / ROWS, cell % ROWS)
}

/// LED code bit: full brightness (else dimmed).
pub const LED_BRIGHT: u8 = 0x20;

/// Pad LED code for controllers: 0 = off, otherwise `1 + colour index`
/// (`1..=17`) plus [`LED_BRIGHT`]. Fits a MIDI velocity.
pub fn led_code(color: u8, bright: bool) -> u8 {
    let c = color.min(COLORS.len() as u8 - 1) + 1;
    if bright { c | LED_BRIGHT } else { c }
}

/// Linear RGB (`0..=1` per channel) for an LED code; dimmed codes at about a
/// fifth of full brightness, which reads as "loaded" next to a playing pad.
pub fn led_rgb(code: u8) -> [f32; 3] {
    let c = code & !LED_BRIGHT;
    if c == 0 || usize::from(c) > COLORS.len() {
        return [0.0; 3];
    }
    let rgb = COLORS[usize::from(c - 1)];
    let level = if code & LED_BRIGHT != 0 { 1.0 } else { 0.18 };
    let ch = |shift: u32| {
        // Rough gamma: LEDs look washed out with sRGB values as they are.
        let v = ((rgb >> shift) & 0xff) as f32 / 255.0;
        v * v * level
    };
    [ch(16), ch(8), ch(0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_map_to_cells_by_page() {
        assert_eq!(pad_cell(1, 0), Some(0));
        assert_eq!(pad_cell(2, 0), Some(ROWS));
        assert_eq!(pad_cell(4, 0), Some(3 * ROWS));
        assert_eq!(pad_cell(5, 0), Some(1));
        assert_eq!(pad_cell(16, 3), Some(3 * ROWS + 15));
        assert_eq!(pad_cell(0, 0), None);
        assert_eq!(pad_cell(17, 0), None);
        assert_eq!(cell_slot_row(3 * ROWS + 15), (3, 15));
    }

    #[test]
    fn led_codes() {
        assert_eq!(led_rgb(0), [0.0; 3]);
        let red = led_rgb(led_code(1, true));
        assert!(red[0] > 0.8 && red[1] < 0.1 && red[2] < 0.1, "{red:?}");
        let dim = led_rgb(led_code(1, false));
        assert!(dim[0] < red[0] * 0.3 && dim[0] > 0.0);
        assert!(led_code(16, true) < 128);
        assert_eq!(led_rgb(led_code(200, false)), led_rgb(led_code(16, false)));
    }
}
