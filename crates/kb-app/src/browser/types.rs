//! Types shared by the browser: drawing updates, the cached layers, page and tab state, sheets.

use super::host::{ChromeSlots, Host};


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

/// A chrome layer on screen: its picture (`gray`, empty until the first render), the key it was drawn for, and the
/// chrome worker that holds its document (`slot`). The picture stays until the new one arrives.
pub(super) struct Cached {
    pub(super) key: String,
    pub(super) gray: Vec<u8>,
    pub(super) slot: usize,
    pub(super) host: Host<ChromeSlots>,
}

impl Cached {
    /// Run `f` on this layer's document on the chrome worker. Waits behind a render of this layer that is running.
    fn on_page<R: Send + 'static>(&self, f: impl FnOnce(&mut kb_engine::Page) -> R + Send + 'static) -> Option<R> {
        let slot = self.slot;
        self.host
            .ask(move |state: &mut ChromeSlots| state.pages.get_mut(slot).and_then(|p| p.as_mut()).map(f))
            .flatten()
    }

    pub(super) fn link_at(&self, x: f32, y: f32) -> Option<String> {
        self.on_page(move |p| p.link_at(x, y)).flatten()
    }

    pub(super) fn link_box(&self, x: f32, y: f32) -> Option<(f32, f32, f32, f32)> {
        self.on_page(move |p| p.link_box(x, y)).flatten()
    }

    pub(super) fn element_box(&self, selector: &str) -> Option<(f32, f32, f32, f32)> {
        let selector = selector.to_string();
        self.on_page(move |p| p.element_box(&selector)).flatten()
    }
}

/// The chrome layer slots (indexes on the chrome worker). The keyboard keys are `KEY0 + layer`.
pub(super) const TOP: usize = 0;
pub(super) const BOTTOM: usize = 1;
pub(super) const SHEET: usize = 2;
pub(super) const SHEET_HEADER: usize = 3;
pub(super) const QUICK: usize = 4;
pub(super) const FIELD: usize = 5;
pub(super) const SUGGEST: usize = 6;
pub(super) const KEY0: usize = 7;
pub(super) const TOAST: usize = 11;

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
