//! Tabs: each tab keeps its own history, scroll position and source. Only the active tab holds a
//! live page.

use super::*;

impl Browser {
    /// (title, url) for each tab, the active one included.
    pub(super) fn tab_labels(&self) -> Vec<(String, String)> {
        (0..self.tabs.len().max(1))
            .map(|i| {
                if i == self.active {
                    (self.title().to_string(), self.url.clone())
                } else {
                    let t = &self.tabs[i];
                    (t.title.clone(), t.url.clone())
                }
            })
            .collect()
    }

    /// Save the active tab's navigation state into its slot.
    pub(super) fn stash_active(&mut self) {
        let title = self.title().to_string();
        self.remember_scroll();
        let state = TabSave {
            history: std::mem::take(&mut self.history),
            scrolls: std::mem::take(&mut self.scrolls),
            idx: self.idx,
            url: self.url.clone(),
            title,
        };
        if self.tabs.len() <= self.active {
            self.tabs.resize_with(self.active + 1, TabSave::empty);
        }
        self.tabs[self.active] = state;
    }

    pub(super) fn switch_tab(&mut self, n: usize) {
        if n >= self.tabs.len() || n == self.active {
            self.sheet = None;
            return;
        }
        self.stash_active();
        let t = std::mem::replace(&mut self.tabs[n], TabSave::empty());
        self.history = t.history;
        self.scrolls = t.scrolls;
        self.idx = t.idx;
        self.url = t.url.clone();
        self.active = n;
        self.sheet = None;
        // Tabs keep navigation metadata; only the active tab owns a live document.
        // Never reuse a different tab's DOM or its live-page cache, even for the same URL.
        self.host.ask(|slots| slots.page = None);
        self.forget_previous();
        self.page_url.clear();
        // The HTML on screen belonged to the previous tab. Clear it so a load that is cancelled
        // leaves nothing to save or reflow under this tab's address.
        self.source.clear();
        self.reader_source = None;
        self.reader = false;
        let url = t.url;
        if url.is_empty() {
            self.open("kbp://home");
        } else {
            // Read the saved position first: loading saves the session, which records the new page's
            // position (the top) in this slot.
            let saved = self.scrolls.get(self.idx).copied().unwrap_or(0.0);
            // The position is restored when the page is shown (the load returns at once).
            let _ = self.load_then(
                &url,
                Policy::Cache(Duration::from_secs(60 * 60 * 24 * 7)),
                Box::new(move |b: &mut Browser| {
                    b.page_on(move |p| p.scroll_to(saved));
                    b.mark_page_dirty();
                    b.touch(0, b.height, Wave::Text);
                }),
            );
        }
    }

    pub(super) fn new_tab(&mut self) {
        self.tabs.push(TabSave::empty());
        let n = self.tabs.len() - 1;
        self.switch_tab(n);
        if self.url.is_empty() {
            self.open("kbp://home");
        }
    }

    pub(super) fn close_tab(&mut self, n: usize) {
        if n >= self.tabs.len() {
            return;
        }
        let sheet = self.sheet;
        if self.tabs.len() == 1 {
            self.history.clear();
            self.scrolls.clear();
            self.idx = 0;
            self.open("kbp://home");
            self.forget_previous();
        } else if n == self.active {
            let next = if n + 1 < self.tabs.len() {
                n + 1
            } else {
                n - 1
            };
            self.switch_tab(next);
            let removed = n;
            self.tabs.remove(removed);
            if removed < self.active {
                self.active -= 1;
            }
        } else {
            self.tabs.remove(n);
            if n < self.active {
                self.active -= 1;
            }
        }
        self.sheet = sheet;
        self.save_session();
        self.touch(0, self.height, Wave::Text);
    }
}
