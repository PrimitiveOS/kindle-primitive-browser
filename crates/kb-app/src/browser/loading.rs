//! Loading documents: fetching and building a page, forms (POST), fragments, reader view, and
//! recording visits.

use super::*;
use crate::ui;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn load_reason(err: &str) -> String {
    if err.contains("HTTP 404") {
        "The address was not found on the server (404).".into()
    } else if err.contains("HTTP") {
        format!(
            "The server answered with an error ({}).",
            err.trim_start_matches("HTTP ")
        )
    } else if err.contains("request failed") {
        "No connection to the site. Check the Wi-Fi and try again.".into()
    } else if err.contains("resolves to this device") {
        "The device sends this site to its own address (a hosts-file or DNS override), so it cannot be reached. Removing that override restores access.".into()
    } else if err.contains("read failed") {
        "The page is too large to open on this device.".into()
    } else {
        "The page could not be loaded.".into()
    }
}

/// The address without its fragment: the document an address belongs to.
pub(super) fn document_of(url: &str) -> String {
    url.split('#').next().unwrap_or(url).to_string()
}

/// The host name of an address, or an empty string.
pub(super) fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
}


/// What to do once a page is shown (after a load or a rebuild).
pub(super) type After = Box<dyn FnOnce(&mut Browser)>;

/// Which document a download fetches.
pub(super) enum FetchJob {
    Get(Policy),
    Post(Vec<(String, String)>),
}

/// A load or a rebuild that has not finished. The UI thread handles input while it runs.
pub(super) enum Pending {
    /// A document downloading on its own thread.
    Fetch {
        rx: mpsc::Receiver<Result<kb_net::Fetched, String>>,
        url: String,
        post: bool,
        after: After,
    },
    /// A page being built on the page worker. `cancel` stops the wait for its images (a tap sets it).
    Build {
        ticket: u64,
        cancel: Arc<AtomicBool>,
        html: String,
        base: String,
        is_web: bool,
        navigate: bool,
        after: After,
    },
}

/// Run `first`, then `second`.
fn chain(first: After, second: After) -> After {
    Box::new(move |b: &mut Browser| {
        first(b);
        second(b);
    })
}

impl Browser {
    /// POST a form and show the response. Returns at once; the response is shown when it has arrived.
    pub(super) fn post_document(
        &mut self,
        url: &str,
        fields: Vec<(String, String)>,
    ) -> Result<(), Cancelled> {
        self.url = url.to_string();
        self.keyboard = false;
        self.sheet = None;
        self.chrome = true;
        self.mark_page_dirty();
        self.start_fetch(url.to_string(), FetchJob::Post(fields), Box::new(|_| {}));
        Ok(())
    }

    /// Load `url` into the page. Returns at once: the document downloads on its own thread and the page is built on
    /// the page worker; `after` runs when the page is shown. The page on screen stays until then.
    pub(super) fn load(&mut self, url: &str, policy: Policy) -> Result<(), Cancelled> {
        self.load_then(url, policy, Box::new(|_| {}))
    }

    /// As `load`, and `after` runs when the new page is shown (restores a reading position, for example).
    pub(super) fn load_then(&mut self, url: &str, policy: Policy, after: After) -> Result<(), Cancelled> {
        self.url = url.to_string();
        self.keyboard = false;
        self.sheet = None;
        self.chrome = true;
        self.mark_page_dirty();
        // Show the address bar with the new URL (and the progress strip under it) before waiting.
        self.touch(0, self.height, Wave::Text);
        self.emit_frame();

        if let Some(rest) = url.strip_prefix("kbp://") {
            // Saved pages resolve relative links against their original address.
            let base = rest
                .strip_prefix("saved/")
                .and_then(|key| {
                    read_pairs(&data_file("saved.tsv"))
                        .into_iter()
                        .find(|(k, _)| k == key)
                })
                .map(|(_, meta)| split_saved_meta(&meta).0.to_string())
                .unwrap_or_else(|| url.to_string());
            let html = self.internal_html(rest);
            self.start_build(&html, &base, false, true, after);
            return Ok(());
        }
        self.start_fetch(url.to_string(), FetchJob::Get(policy), after);
        Ok(())
    }

    /// Download a document on its own thread. The thread wakes the UI (`set_waker`) when it has finished.
    pub(super) fn start_fetch(&mut self, url: String, job: FetchJob, after: After) {
        let (tx, rx) = mpsc::channel();
        let wake = self.waker.clone();
        let post = matches!(job, FetchJob::Post(_));
        let target = url.clone();
        thread::spawn(move || {
            let result = match job {
                FetchJob::Get(policy) => kb_net::load(&target, MAX_DOC_BYTES, policy),
                FetchJob::Post(fields) => kb_net::post_form(&target, &fields, MAX_DOC_BYTES),
            };
            let _ = tx.send(result);
            if let Some(wake) = wake {
                wake();
            }
        });
        self.set_pending(Pending::Fetch { rx, url, post, after });
    }

    /// Replace the pending load. A build that it replaces is cancelled (its images are not waited for).
    fn set_pending(&mut self, next: Pending) {
        if let Some(Pending::Build { cancel, .. }) = &self.pending {
            cancel.store(true, Ordering::Relaxed);
        }
        self.pending = Some(next);
    }

    /// The user touched the screen or pressed a button: stop waiting for the images of a page being built.
    pub(super) fn interrupt_loading(&mut self) {
        if let Some(Pending::Build { cancel, .. }) = &self.pending {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The progress strip's fraction while a load runs (`None` when nothing loads).
    pub(super) fn load_progress(&self) -> Option<f32> {
        match &self.pending {
            None => None,
            Some(Pending::Fetch { .. }) => Some(0.25),
            Some(Pending::Build { .. }) => {
                let s = self.net.stats();
                let req = s.requested.load(Ordering::Relaxed);
                let done = s.done.load(Ordering::Relaxed);
                Some(if req == 0 { 0.3 } else { 0.3 + 0.6 * done as f32 / req as f32 })
            }
        }
    }

    /// Take the download that has arrived, or the page build that has finished. Returns true when one did.
    pub(super) fn poll_pending(&mut self) -> bool {
        match self.pending.take() {
            None => false,
            Some(Pending::Fetch { rx, url, post, after }) => match rx.try_recv() {
                Ok(result) => {
                    self.finish_fetch(url, post, result, after);
                    true
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.pending = Some(Pending::Fetch { rx, url, post, after });
                    false
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finish_fetch(url, post, Err("network thread stopped".to_string()), after);
                    true
                }
            },
            Some(Pending::Build { ticket, cancel, html, base, is_web, navigate, after }) => {
                if self.host.take::<()>(ticket).is_some() {
                    self.finish_build(html, base, is_web, navigate, after);
                    true
                } else {
                    self.pending = Some(Pending::Build { ticket, cancel, html, base, is_web, navigate, after });
                    false
                }
            }
        }
    }

    /// Block until the pending load or rebuild is finished and shown.
    pub(super) fn wait_pending(&mut self) {
        match self.pending.take() {
            None => {}
            Some(Pending::Fetch { rx, url, post, after }) => {
                let result = rx.recv().unwrap_or_else(|_| Err("network thread stopped".to_string()));
                self.finish_fetch(url, post, result, after);
            }
            Some(Pending::Build { ticket, html, base, is_web, navigate, after, .. }) => {
                let _ = self.host.wait::<()>(ticket);
                self.finish_build(html, base, is_web, navigate, after);
            }
        }
    }

    /// The download has arrived: build the page it holds (or the error page).
    fn finish_fetch(&mut self, url: String, post: bool, result: Result<kb_net::Fetched, String>, after: After) {
        match result {
            Ok(f) => {
                if post {
                    self.url = f.url.clone();
                    if let Some(last) = self.history.get_mut(self.idx) {
                        *last = f.url.clone();
                    }
                    let html = String::from_utf8_lossy(&f.body).into_owned();
                    self.start_build(&html, &f.url, true, true, after);
                    return;
                }
                // HTTP does not send the fragment; keep it when the response URI omits it.
                let mut final_url = f.url.clone();
                if let (Ok(requested), Ok(mut parsed)) = (url::Url::parse(&url), url::Url::parse(&f.url)) {
                    if parsed.fragment().is_none() {
                        parsed.set_fragment(requested.fragment());
                        final_url = parsed.to_string();
                    }
                }
                self.url = final_url.clone();
                if let Some(last) = self.history.get_mut(self.idx) {
                    *last = final_url.clone();
                }
                let html = if !f.content_type.is_empty() && !f.content_type.contains("html") {
                    ui::error(
                        "Not a web page",
                        "This link opens a file that is not a web page.",
                        &f.content_type,
                    )
                } else {
                    String::from_utf8_lossy(&f.body).into_owned()
                };
                self.start_build(&html, &final_url, true, true, after);
            }
            Err(e) => {
                eprintln!("[{}] {url}: {e}", if post { "post" } else { "load" });
                let html = ui::error("Could not load this page", &load_reason(&e), &e);
                self.start_build(&html, &url, false, true, after);
            }
        }
    }

    /// Rebuild the page on screen from HTML (reflow, reader view). `after` runs when it is shown.
    pub(super) fn build_then(&mut self, html: &str, base: &str, is_web: bool, after: After) {
        self.start_build(html, base, is_web, false, after);
    }

    /// Build a page on the page worker. The page on screen goes into the one-page cache first, as before; the worker
    /// moves it. Returns at once; `finish_build` shows the page when the build has finished.
    pub(super) fn start_build(&mut self, html: &str, base: &str, is_web: bool, navigate: bool, after: After) {
        self.find_matches.clear();
        self.page_url = base.to_string();
        // Web pages show alt text in place of images when images are off; browser pages keep their images.
        let scripts = !is_web || !crate::storage::scripts_off(&host_of(base));
        let prepared = prepare_html(html, &self.settings, is_web && !self.settings.images, scripts);
        // Stylesheets for print or the dark scheme are not applied to this screen.
        let prepared = if is_web { crate::html::drop_unmatched_stylesheet_links(&prepared) } else { prepared };
        let (w, h) = (self.width, self.height);
        // Web pages lay out at a phone width; internal pages and chrome keep device pixels.
        let css_width = if is_web { Some(self.settings.page_width as f32) } else { None };
        // The browser's own pages (home with its logo) always show their images.
        self.net.set_images_allowed(self.settings.images || !is_web);

        // The page on screen is kept for Back when it is another document (the worker moves it).
        let keep = self.has_page() && !self.page_url.is_empty() && self.page_url != self.url;
        if keep {
            self.previous = Some(PreviousPage {
                url: self.page_url.clone(),
                source: std::mem::take(&mut self.source),
                reader_source: self.reader_source.take(),
                is_web: self.page_is_web,
            });
        }
        self.page_url = self.url.clone();
        self.source = html.to_string();
        self.reader_source = None;
        self.reader = false;
        self.page_is_web = is_web;
        self.progress = Some(0.3);

        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let html_owned = prepared;
        let base_owned = base.to_string();
        let raw_html = html.to_string();
        let ticket = self.host.post(move |slots: &mut Slots| {
            let current = slots.page.take();
            if keep {
                slots.previous = current;
            }
            let net = slots.net.clone();
            net.set_defer_images(true);
            let mut waiting = || !flag.load(Ordering::Relaxed);
            let built = std::panic::catch_unwind(AssertUnwindSafe(|| {
                Page::new_scaled(&html_owned, &base_owned, w, h, net.clone(), &mut waiting, css_width)
            }));
            net.set_defer_images(false);
            let page = match built {
                Ok(p) => p,
                Err(_) => {
                    let fallback = ui::error("Page error", "This page could not be shown.", "");
                    Page::new(&fallback, &base_owned, w, h, net.clone(), &mut || true)
                }
            };
            page.start_deferred_images();
            slots.page = Some(page);
        });
        match ticket {
            Some(ticket) => self.set_pending(Pending::Build {
                ticket,
                cancel,
                html: raw_html,
                base: base.to_string(),
                is_web,
                navigate,
                after,
            }),
            None => {
                eprintln!("[page] the page worker is gone; the page is not shown");
                self.progress = None;
            }
        }
    }

    /// The page is built: show it. For a navigation, also record the visit, follow a meta refresh, scroll to the
    /// fragment, follow script navigation, save the session; then `after`.
    fn finish_build(&mut self, html: String, base: String, is_web: bool, navigate: bool, after: After) {
        self.progress = None;
        self.mark_page_dirty();
        if navigate {
            if is_web {
                self.record_web_visit();
                self.pending_nav = crate::html::meta_refresh_target(&html, &base);
            }
            if let Some(fragment) = url::Url::parse(&self.url)
                .ok()
                .and_then(|u| u.fragment().map(str::to_owned))
            {
                self.page_on(move |p| p.scroll_to_fragment(&fragment));
            }
            self.follow_script_navigation();
            self.save_session();
        }
        // A finished page gets one full refresh (clears the previous page's ghosting), unless the reader turned it off.
        let refresh = if self.settings.flash_on_load { Wave::Flash } else { Wave::Text };
        self.touch(0, self.height, refresh);
        // A navigation that the page started (a script or a redirect) has its own pending load: the continuation
        // waits for it, so the position is restored on the page it belongs to.
        match self.pending.as_mut() {
            Some(Pending::Fetch { after: next, .. }) | Some(Pending::Build { after: next, .. }) => {
                let old = std::mem::replace(next, Box::new(|_| {}));
                *next = chain(after, old);
            }
            None => after(self),
        }
    }

    pub(super) fn follow_script_navigation(&mut self) -> bool {
        // history.pushState/replaceState: show each new address in order, and keep the history entries current.
        for (url, replace, state) in self.page_on(|p| p.take_url_changes()).unwrap_or_default() {
            self.url = url.clone();
            self.pushed_states.insert(url.clone(), (document_of(&self.page_url), state));
            if replace {
                if let Some(last) = self.history.get_mut(self.idx) {
                    *last = url;
                }
            } else {
                // pushState adds an entry after the current one (dropping any forward entries), as browsers do.
                self.history.truncate(self.idx + 1);
                self.history.push(url);
                self.idx = self.history.len() - 1;
            }
            self.save_session();
        }
        if let Some(next) = self.page_on(|p| p.take_navigation()).flatten() {
            self.pending_nav = Some(next);
        }
        let Some(next) = self.pending_nav.take() else { return false };
        if self.redirect_depth >= 5 {
            return false;
        }
        if same_document(&self.url, &next) && self.url != next {
            self.show_fragment(&next);
            return false;
        }
        self.redirect_depth += 1;
        let _ = self.load(&next, Policy::Network);
        self.redirect_depth -= 1;
        true
    }

    /// Switch between the page and its reader view (article markup, laid out at the chosen width).
    pub(super) fn toggle_reader(&mut self) {
        if self.reader {
            let url = self.url.clone();
            let _ = self.load(&url, Policy::Cache(Duration::from_secs(60 * 60 * 24 * 7)));
            return;
        }
        let Some(html) = self.page_on(|p| p.reader_html()).flatten() else {
            return;
        };
        let base = self.url.clone();
        let source = self.source.clone();
        let reader_html = html.clone();
        self.build_then(
            &html,
            &base,
            true,
            Box::new(move |b: &mut Browser| {
                b.source = source;
                b.reader_source = Some(reader_html);
                b.reader = true;
                b.touch(0, b.height, Wave::Text);
            }),
        );
    }

    /// Record a web page the reader navigated to (history and cookies). Rebuilds such as
    /// reflow or reader view lay the page out again without recording it a second time.
    pub(super) fn record_web_visit(&mut self) {
        let url = self.url.clone();
        let title = self.title().to_string();
        self.record_visit(&url, &title);
        kb_net::save_cookies();
    }
}
