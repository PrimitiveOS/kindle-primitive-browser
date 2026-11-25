//! Find in page: a query typed on the keyboard, the matches on the page, and moving from one match to the next.
//! Each Find (Go) moves to the next match of the same query; a new query starts at the first match.

use super::*;

impl Browser {
    /// Open the keyboard for a find. The last query is selected, so typing replaces it.
    pub(super) fn begin_find(&mut self) {
        self.find_entry = true;
        self.keyboard = true;
        self.sheet = None;
        self.typed = self.find_query.chars().collect();
        self.caret = self.typed.len();
        self.select_all = !self.typed.is_empty();
        self.layer = Layer::Lower;
        self.refresh_suggestions();
        self.touch(0, self.height, Wave::Text);
    }

    /// Search the page for `query`, scroll to a match and say which one it is.
    pub(super) fn run_find(&mut self, query: &str) {
        let query = query.trim().to_string();
        if query.is_empty() {
            self.touch(0, self.height, Wave::Text);
            return;
        }
        if query == self.find_query && !self.find_matches.is_empty() {
            self.find_index = (self.find_index + 1) % self.find_matches.len();
        } else {
            let needle = query.clone();
            self.find_matches = self.page_on(move |p| p.find_matches(&needle)).unwrap_or_default();
            self.find_query = query.clone();
            self.find_index = 0;
        }
        if self.find_matches.is_empty() {
            self.notify(&format!("No matches for \"{query}\""));
            self.touch(0, self.height, Wave::Text);
            return;
        }
        let y = self.find_matches[self.find_index];
        self.page_on(move |p| p.scroll_to_match(y));
        self.mark_page_dirty();
        self.notify(&format!(
            "Match {} of {}. Find again for the next one.",
            self.find_index + 1,
            self.find_matches.len()
        ));
        self.touch(0, self.height, Wave::Text);
    }
}
