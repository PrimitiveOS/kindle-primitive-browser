//! Sessions: the open tabs, each tab's history and scroll positions, and which tab is active.
//! Saved whenever a page or tab changes and when the browser leaves, and restored when it opens
//! again, so leaving the browser (Exit, or a crash) does not lose the tabs.

use super::*;

const SESSION_FILE: &str = "session.tsv";

/// Keep a field on one line and free of the tab separator.
fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// The session as text: `active <n>`, then for each tab `tab <idx> <url> <title>` followed by one
/// `h <scroll> <url>` line per history entry.
pub(super) fn encode_session(tabs: &[TabSave], active: usize) -> String {
    let mut out = format!("active\t{active}\n");
    for t in tabs {
        out.push_str(&format!("tab\t{}\t{}\t{}\n", t.idx, clean(&t.url), clean(&t.title)));
        for (i, url) in t.history.iter().enumerate() {
            let scroll = t.scrolls.get(i).copied().unwrap_or(0.0);
            out.push_str(&format!("h\t{scroll}\t{}\n", clean(url)));
        }
    }
    out
}

/// Read a session written by `encode_session`. `None` when it holds no tab.
pub(super) fn decode_session(text: &str) -> Option<(Vec<TabSave>, usize)> {
    let mut tabs: Vec<TabSave> = Vec::new();
    let mut active = 0usize;
    for line in text.lines() {
        let mut f = line.split('\t');
        match f.next() {
            Some("active") => active = f.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            Some("tab") => {
                let idx = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                let url = f.next().unwrap_or("").to_string();
                let title = f.next().unwrap_or("").to_string();
                tabs.push(TabSave {
                    history: Vec::new(),
                    scrolls: Vec::new(),
                    idx,
                    url,
                    title,
                });
            }
            Some("h") => {
                let scroll = f.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                let url = f.next().unwrap_or("").to_string();
                if let Some(t) = tabs.last_mut() {
                    t.history.push(url);
                    t.scrolls.push(scroll);
                }
            }
            _ => {}
        }
    }
    (!tabs.is_empty()).then_some((tabs, active))
}

impl Browser {
    /// The active tab's state as it is now. Unlike `stash_active`, the live history stays put.
    fn live_tab(&mut self) -> TabSave {
        self.remember_scroll();
        TabSave {
            history: self.history.clone(),
            scrolls: self.scrolls.clone(),
            idx: self.idx,
            url: self.url.clone(),
            title: self.title().to_string(),
        }
    }

    /// Write the open tabs to the data folder.
    pub(super) fn save_session(&mut self) {
        let live = self.live_tab();
        let mut tabs = self.tabs.clone();
        if tabs.len() <= self.active {
            tabs.resize_with(self.active + 1, TabSave::empty);
        }
        tabs[self.active] = live;
        let path = data_file(SESSION_FILE);
        let _ = crate::storage::write_atomic(&path, encode_session(&tabs, self.active).as_bytes());
    }

    /// Open the browser: the tabs saved last time come back, each with its history and scroll
    /// position. Without a saved session, `start` is opened.
    pub fn resume(&mut self, start: &str) {
        let saved = fs::read_to_string(data_file(SESSION_FILE))
            .ok()
            .and_then(|text| decode_session(&text));
        let Some((mut tabs, active)) = saved else {
            self.open(start);
            return;
        };
        let active = active.min(tabs.len() - 1);
        let t = std::mem::replace(&mut tabs[active], TabSave::empty());
        self.tabs = tabs;
        self.active = active;
        self.history = t.history;
        self.scrolls = t.scrolls;
        self.url = t.url.clone();
        if self.history.is_empty() && !t.url.is_empty() {
            self.history.push(t.url.clone());
        }
        self.scrolls.resize(self.history.len(), 0.0);
        self.idx = t.idx.min(self.history.len().saturating_sub(1));
        if t.url.is_empty() {
            self.open(start);
            return;
        }
        // Restore from the cache first, so the browser comes back quickly even on a slow link.
        // The position is read before loading, which would record the top of the new page.
        let saved_scroll = self.scrolls.get(self.idx).copied().unwrap_or(0.0);
        let _ = self.load(&t.url, Policy::Cache(Duration::from_secs(60 * 60 * 24 * 7)));
        self.page_on(move |p| p.scroll_to(saved_scroll));
        self.mark_page_dirty();
        self.touch(0, self.height, Wave::Text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(url: &str, title: &str, history: &[(&str, f64)], idx: usize) -> TabSave {
        TabSave {
            history: history.iter().map(|(u, _)| u.to_string()).collect(),
            scrolls: history.iter().map(|(_, s)| *s).collect(),
            idx,
            url: url.to_string(),
            title: title.to_string(),
        }
    }

    #[test]
    fn session_round_trips_tabs_history_and_scroll() {
        let tabs = vec![
            tab("https://a.test/x", "A", &[("https://a.test/", 0.0), ("https://a.test/x", 430.5)], 1),
            tab("kbp://bookmarks", "Bookmarks", &[("kbp://bookmarks", 0.0)], 0),
        ];
        let text = encode_session(&tabs, 1);
        let (back, active) = decode_session(&text).expect("a session");
        assert_eq!(active, 1);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].history, vec!["https://a.test/", "https://a.test/x"]);
        assert_eq!(back[0].scrolls, vec![0.0, 430.5]);
        assert_eq!(back[0].idx, 1);
        assert_eq!(back[1].url, "kbp://bookmarks");
    }

    #[test]
    fn tabs_and_titles_cannot_break_the_line_format() {
        let tabs = vec![tab("https://a.test/", "Two\tparts\nhere", &[("https://a.test/", 0.0)], 0)];
        let (back, _) = decode_session(&encode_session(&tabs, 0)).expect("a session");
        assert_eq!(back[0].title, "Two parts here");
    }

    #[test]
    fn empty_or_garbled_session_restores_nothing() {
        assert!(decode_session("").is_none());
        assert!(decode_session("active\t0\nh\t1\thttps://orphan.test/\n").is_none());
    }
}
