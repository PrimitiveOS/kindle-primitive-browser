//! Navigation: opening pages, Back and Forward, history, reflow after a setting change.

use super::*;

impl Browser {
    /// Navigate to a user-typed address, search, or internal page.
    pub fn open(&mut self, input: &str) {
        let target = normalize(input);
        self.remember_scroll();
        let prev = self.idx;
        let had_history = !self.history.is_empty();
        self.history.truncate(self.idx + usize::from(had_history));
        self.scrolls.truncate(self.history.len());
        self.history.push(target.clone());
        self.scrolls.push(0.0);
        self.idx = self.history.len() - 1;
        if self.has_page() && same_document(&self.url, &target) && self.url != target {
            self.show_fragment(&target);
            return;
        }
        if self.load(&target, Policy::Network).is_err() {
            // Cancelled by the user before it arrived: undo the history entry.
            self.history.pop();
            self.scrolls.pop();
            self.idx = prev;
        }
    }

    pub fn back(&mut self) {
        if self.can_go_back() {
            self.remember_scroll();
            let prev = self.idx;
            self.idx -= 1;
            self.load_history_entry(prev);
        }
    }

    pub fn forward(&mut self) {
        if self.can_go_forward() {
            self.remember_scroll();
            let prev = self.idx;
            self.idx += 1;
            self.load_history_entry(prev);
        }
    }

    pub fn reload(&mut self) {
        if !self.url.is_empty() {
            let url = self.url.clone();
            let _ = self.load(&url, Policy::Network);
        }
    }

    pub(super) fn load_history_entry(&mut self, prev: usize) {
        let url = self.history[self.idx].clone();
        let saved = self.scrolls.get(self.idx).copied().unwrap_or(0.0);
        // An entry that pushState created on the page still loaded: no reload, just popstate with its state.
        if let Some((doc, state)) = self.pushed_states.get(&url).cloned() {
            if self.has_page() && doc == super::loading::document_of(&self.page_url) {
                self.url = url.clone();
                let location = url.clone();
                self.page_on(move |p| {
                    p.set_location(&location);
                    p.pop_state(&state);
                    p.scroll_to(saved);
                });
                self.mark_page_dirty();
                self.keyboard = false;
                self.sheet = None;
                self.touch(0, self.height, Wave::Text);
                return;
            }
        }
        if self.has_page() && same_document(&self.url, &url) && self.url != url {
            self.show_fragment(&url);
            self.page_on(move |p| p.scroll_to(saved));
            return;
        }
        if self.restore_previous(&url) {
            self.page_on(move |p| p.scroll_to(saved));
            return;
        }
        self.navigating_history = true;
        let outcome = self.load(&url, Policy::Cache(Duration::from_secs(60 * 60 * 24 * 7)));
        self.navigating_history = false;
        match outcome {
            Ok(()) => {
                self.page_on(move |p| p.scroll_to(saved));
            }
            Err(Cancelled) => self.idx = prev,
        }
    }

    pub(super) fn show_fragment(&mut self, url: &str) {
        let location = url.to_string();
        let fragment = url::Url::parse(url)
            .ok()
            .and_then(|u| u.fragment().map(str::to_owned))
            .unwrap_or_default();
        self.page_on(move |p| {
            p.set_location(&location);
            p.scroll_to_fragment(&fragment);
        });
        self.url = url.to_string();
        self.page_url = url.to_string();
        self.keyboard = false;
        self.sheet = None;
        self.chrome = true;
        self.mark_page_dirty();
        self.touch(0, self.height, Wave::Text);
    }

    /// Carry out a submitted form: GET navigates to the query URL, POST sends the fields.
    pub(super) fn submit(&mut self, submit: kb_engine::Submit) {
        match submit {
            kb_engine::Submit::Get(url) => self.open(&url),
            kb_engine::Submit::Post { url, fields } => {
                self.remember_scroll();
                let prev = self.idx;
                let had_history = !self.history.is_empty();
                self.history.truncate(self.idx + usize::from(had_history));
                self.scrolls.truncate(self.history.len());
                self.history.push(url.clone());
                self.scrolls.push(0.0);
                self.idx = self.history.len() - 1;
                if self.post_document(&url, fields).is_err() {
                    self.history.pop();
                    self.scrolls.pop();
                    self.idx = prev;
                }
            }
        }
    }

    pub(super) fn stash_current(&mut self, next_url: &str) {
        if self.has_page() {
            let keep = !self.page_url.is_empty() && self.page_url != next_url;
            if keep {
                self.previous = Some(PreviousPage {
                    url: self.page_url.clone(),
                    source: std::mem::take(&mut self.source),
                    reader_source: self.reader_source.take(),
                    is_web: self.page_is_web,
                });
            }
            // The page worker moves the document the same way: kept for Back, or dropped.
            self.host.ask(move |slots| {
                let page = slots.page.take();
                if keep {
                    slots.previous = page;
                }
            });
        }
        self.page_url = next_url.to_string();
    }

    /// Drop the page kept for Back (its document and its address).
    pub(super) fn forget_previous(&mut self) {
        self.previous = None;
        self.host.ask(|slots| slots.previous = None);
    }

    pub(super) fn restore_previous(&mut self, url: &str) -> bool {
        let Some(previous) = &self.previous else {
            return false;
        };
        if previous.url != url && !same_document(&previous.url, url) {
            return false;
        }
        let Some(previous) = self.previous.take() else {
            return false;
        };
        // The page on screen goes into the cache (as `stash_current` does), and the kept page comes back.
        let keep = self.has_page() && !self.page_url.is_empty() && self.page_url != url;
        if keep {
            self.previous = Some(PreviousPage {
                url: self.page_url.clone(),
                source: std::mem::take(&mut self.source),
                reader_source: self.reader_source.take(),
                is_web: self.page_is_web,
            });
        }
        let location = url.to_string();
        self.host.ask(move |slots| {
            let current = slots.page.take();
            let kept = slots.previous.take();
            if keep {
                slots.previous = current;
            }
            slots.page = kept;
            if let Some(page) = slots.page.as_mut() {
                page.set_location(&location);
            }
        });
        self.page_url = url.to_string();
        self.source = previous.source;
        self.reader_source = previous.reader_source;
        self.page_is_web = previous.is_web;
        self.page_url = url.to_string();
        self.url = url.to_string();
        self.keyboard = false;
        self.sheet = None;
        self.chrome = true;
        self.reader = self.reader_source.is_some();
        self.mark_page_dirty();
        self.touch(0, self.height, Wave::Text);
        true
    }

    pub(super) fn remember_scroll(&mut self) {
        let pos = self.page_on(|p| p.scroll_y()).unwrap_or(0.0);
        if let Some(slot) = self.scrolls.get_mut(self.idx) {
            *slot = pos;
        }
    }

    pub(super) fn reflow(&mut self) {
        // A cached layout belongs to the previous display/image settings.
        self.forget_previous();
        let url = self.url.clone();
        if url.is_empty()
            || url.starts_with("kbp://")
            || url.starts_with("kb://")
            || self.source.is_empty()
        {
            // Reloading an internal page clears the sheet; keep the one the user is adjusting.
            let sheet = self.sheet;
            self.refresh_internal();
            self.sheet = sheet;
            self.touch(0, self.height, Wave::Text);
            return;
        }
        let scroll = self.page_on(|p| p.scroll_y()).unwrap_or(0.0);
        let source = self.source.clone();
        let reader_source = self.reader_source.clone();
        let rendered = reader_source.as_deref().unwrap_or(&source);
        let base = self
            .page_on(|p| p.base_url().to_string())
            .unwrap_or_else(|| url.clone());
        self.build_page(rendered, &base, self.page_is_web);
        self.source = source;
        self.reader = reader_source.is_some();
        self.reader_source = reader_source;
        self.page_on(move |p| p.scroll_to(scroll));
        self.mark_page_dirty();
        self.touch(0, self.height, Wave::Text);
    }

    /// Re-open the internal page on screen (home, bookmarks) so it shows the latest data.
    /// Does not add a history entry.
    pub(super) fn refresh_internal(&mut self) {
        let url = self.url.clone();
        if url.starts_with("kbp://") {
            let _ = self.load(&url, Policy::Network);
        }
    }
}
