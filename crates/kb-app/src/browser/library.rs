//! Bookmarks, visit history and saved pages.

use super::*;

impl Browser {
    pub(super) fn toggle_bookmark(&mut self) {
        if self.url.is_empty() || self.url.starts_with("kbp:") {
            return;
        }
        if let Some(i) = self.bookmarks.iter().position(|(u, _)| *u == self.url) {
            self.bookmarks.remove(i);
        } else {
            self.bookmarks
                .push((self.url.clone(), self.title().to_string()));
        }
        write_pairs(&data_file("bookmarks.tsv"), &self.bookmarks);
        let added = self.bookmarks.iter().any(|(u, _)| *u == self.url);
        self.notify(if added {
            "Bookmark added"
        } else {
            "Bookmark removed"
        });
        self.touch_chrome(self.height - BOT_H, self.height, Wave::Fast);
    }

    pub(super) fn record_visit(&mut self, url: &str, title: &str) {
        self.visits.retain(|(u, _)| u != url);
        self.visits.insert(0, (url.to_string(), title.to_string()));
        self.visits.truncate(HISTORY_CAP);
        write_pairs(&data_file("history.tsv"), &self.visits);
    }

    pub(super) fn save_offline(&mut self) {
        if self.url.is_empty() || self.url.starts_with("kbp:") || self.source.is_empty() {
            return;
        }
        let key = format!("{:016x}", fnv(&self.url));
        let dir = data_file("saved");
        // The copy keeps the page's stylesheets and images (budget 6 MB) and drops its scripts.
        let archived = crate::archive::archive_html(
            &self.source,
            &self.url,
            &mut |u| kb_net::fetch(u, 4 * 1024 * 1024).ok().map(|f| (f.content_type, f.body)),
            6 * 1024 * 1024,
        );
        if crate::storage::write_atomic(&dir.join(format!("{key}.html")), archived.as_bytes()).is_err() {
            // The device may be full or read-only: say so, rather than list a page that was not kept.
            self.notify("Could not save this page: the storage did not accept it");
            return;
        }
        let mut index = read_pairs(&data_file("saved.tsv"));
        index.retain(|(k, _)| *k != key);
        index.push((key, format!("{} {}", self.url, self.title())));
        write_pairs(&data_file("saved.tsv"), &index);
        self.sheet_layer = None;
        self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
        self.notify("Saved for offline");
    }
}
