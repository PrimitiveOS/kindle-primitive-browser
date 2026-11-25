//! Types shared by the browser: drawing updates, the cached layers, page and tab state, sheets.

use kb_engine::Page;

pub type Pump = Box<dyn FnMut(f32, &str) -> bool>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Wave {
    /// Fast, low-detail (DU). Bars, keyboard, progress.
    Fast,
    /// Text-quality (GL16). Page content.
    Text,
    /// Full flash (GC16). Clears ghosting.
    Flash,
}

#[derive(Clone, Debug)]
pub struct Update {
    pub gray: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub rows: (u32, u32),
    /// The columns `[x0, x1)` that changed. A tap highlight changes one key, so its update covers only the key's
    /// columns; every other update covers the full width.
    pub cols: (u32, u32),
    pub wave: Wave,
}

impl Update {
    /// Rows `[start, end)` as a standalone gray buffer (full width).
    pub fn region(&self) -> Vec<u8> {
        let (a, b) = self.rows;
        let w = self.width as usize;
        self.gray[a as usize * w..b as usize * w].to_vec()
    }

    /// The changed rectangle: (x, width, gray), where the gray is the rows of the columns `[x, x + width)`.
    pub fn changed_area(&self) -> (u32, u32, Vec<u8>) {
        let (a, b) = self.rows;
        let (x0, x1) = (self.cols.0.min(self.width), self.cols.1.min(self.width));
        let w = self.width as usize;
        let mut out = Vec::with_capacity(((b - a) * (x1 - x0)) as usize);
        for row in a as usize..b as usize {
            out.extend_from_slice(&self.gray[row * w + x0 as usize..row * w + x1 as usize]);
        }
        (x0, x1 - x0, out)
    }
}

#[derive(Debug)]
pub struct Cancelled;

pub(super) struct Cached {
    pub(super) key: String,
    pub(super) page: Page,
    pub(super) gray: Vec<u8>,
}

/// The page kept for Back. Its document is on the page worker (`Slots::previous`).
pub(super) struct PreviousPage {
    pub(super) url: String,
    pub(super) source: String,
    pub(super) reader_source: Option<String>,
    pub(super) is_web: bool,
}

/// A paint or a prefetch sent to the page worker and not yet applied. It is applied only if the page
/// has not changed since (`gen`), and its screen is cached under `key` (scroll position, chrome offset).
#[derive(Clone, Copy, Debug)]
pub(super) struct Flight {
    pub(super) ticket: u64,
    pub(super) gen: u64,
    pub(super) key: (u64, u32),
    /// The chrome offset the picture is painted with (a paint only; a prefetch keeps the key's offset).
    pub(super) offset: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Sheet {
    Menu,
    Tabs,
    Display,
    /// The options of a page's select, to choose one.
    Choice,
}

#[derive(Clone, Default, Debug)]
pub(super) struct TabSave {
    pub(super) history: Vec<String>,
    pub(super) scrolls: Vec<f64>,
    pub(super) idx: usize,
    pub(super) url: String,
    pub(super) title: String,
}

impl TabSave {
    pub(super) fn empty() -> TabSave {
        TabSave::default()
    }
}

#[cfg(test)]
mod update_area_tests {
    use super::*;

    #[test]
    fn changed_area_is_the_columns_and_rows_that_changed() {
        // A 4 x 3 screen: the value of each pixel is its index.
        let gray: Vec<u8> = (0..12).collect();
        let whole = Update { gray: gray.clone(), width: 4, height: 3, rows: (0, 3), cols: (0, 4), wave: Wave::Text };
        assert_eq!(whole.changed_area(), (0, 4, gray.clone()));
        // Columns 1..3 of rows 1..3: the two middle pixels of the last two rows.
        let key = Update { gray, width: 4, height: 3, rows: (1, 3), cols: (1, 3), wave: Wave::Fast };
        assert_eq!(key.changed_area(), (1, 2, vec![5, 6, 9, 10]));
    }
}
