//! Composing the screen: bars, sheets, notices and the keyboard, the cached layers, the notice
//! timer, and PNG output.

use super::*;
use crate::ui;

pub(super) fn blit(dst: &mut [u8], width: u32, y: u32, src: &[u8], rows: u32) {
    let w = width as usize;
    let start = y as usize * w;
    let n = (rows as usize * w)
        .min(src.len())
        .min(dst.len().saturating_sub(start));
    dst[start..start + n].copy_from_slice(&src[..n]);
}

pub(super) fn progress_strip(dst: &mut [u8], width: u32, y: u32, fraction: f32) {
    let w = width as usize;
    let fill = (fraction.clamp(0.0, 1.0) * width as f32) as usize;
    for row in 0..PROGRESS_H as usize {
        let base = (y as usize + row) * w;
        if base + w > dst.len() {
            break;
        }
        for x in 0..w {
            dst[base + x] = if x < fill { 0 } else { 200 };
        }
    }
}

pub fn encode_png(width: u32, height: u32, gray: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(gray).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Add the rows `[y0, y1)` at `wave` to a pending change: the union of rows, the strongest waveform.
fn merge_change(
    change: Option<((u32, u32), Wave)>,
    y0: u32,
    y1: u32,
    wave: Wave,
) -> Option<((u32, u32), Wave)> {
    Some(match change {
        None => ((y0, y1), wave),
        Some(((a, b), w)) => ((a.min(y0), b.max(y1)), w.max(wave)),
    })
}

impl Browser {
    /// Top of the keyboard's keys on this screen.
    pub(super) fn kb_y(&self) -> u32 {
        kb_top(self.height)
    }

    /// Render the screen next to the current one while the reader is idle, so a page turn either way
    /// copies it instead of painting. The paint runs on the page worker; its result is cached in `collect`.
    /// One screen per call.
    pub(super) fn prefetch_next_screen(&mut self) {
        if self.page_dirty || self.paint.is_some() || self.prefetch.is_some() {
            return;
        }
        if self.sheet.is_some() || self.keyboard || self.quick || !self.has_page() {
            return;
        }
        let mirror = self.host.mirror();
        let (current, max) = (mirror.scroll_y, mirror.max_scroll);
        // The same distances a page turn will use, so the turn finds its screen here.
        let next = (current + self.turn_distance(1.0)).min(max);
        let prev = (current + self.turn_distance(-1.0)).max(0.0);
        let mut wanted = Vec::new();
        if next > current {
            wanted.push((next, 0));
        }
        if prev < current {
            // A turn back keeps the bars as they are.
            wanted.push((prev, self.page_offset()));
        }
        for (target, offset) in wanted {
            let key = (target.round() as u64, offset);
            if self.page_cache.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let gen = self.page_gen;
            let ticket = self.host.post(move |slots: &mut Slots| {
                let Some(page) = slots.page.as_mut() else { return Vec::new() };
                page.scroll_to(target);
                let gray = page.render_gray_at(offset);
                page.scroll_to(current);
                gray
            });
            self.prefetch = ticket.map(|ticket| Flight { ticket, gen, key, offset });
            return;
        }
    }

    /// Called regularly by the driver (on idle). Brings in held-back images and clears a notice when its
    /// time is up. A page change it causes is shown when its paint finishes (`page_ready`).
    pub fn tick(&mut self) -> Option<Update> {
        self.collect();
        if self.paint.is_none() && self.prefetch.is_none() {
            self.prefetch_next_screen();
        }
        let done = self
            .net
            .stats()
            .deferred_done
            .load(std::sync::atomic::Ordering::Relaxed);
        // Images trickle in one by one. Repaint at most every 1.5 s so the page doesn't flash.
        if done != self.images_seen
            && self.last_image_refresh.elapsed() >= Duration::from_millis(1500)
        {
            self.images_seen = done;
            self.last_image_refresh = Instant::now();
            // The last image arriving ends the load: refresh the whole screen once, cleanly.
            let wave = if self.images_loading() { Wave::Text } else { Wave::Flash };
            self.touch(0, self.height, wave);
            self.mark_page_dirty();
            return self.try_frame();
        }
        if let Some((_, until)) = &self.toast {
            if Instant::now() >= *until {
                self.toast = None;
                let (y0, y1) = toast_rows(self.height);
                self.touch_chrome(y0, y1, Wave::Fast);
                return self.try_frame();
            }
        }
        None
    }

    pub fn notify(&mut self, message: &str) {
        self.toast = Some((
            message.to_string(),
            Instant::now() + Duration::from_millis(2200),
        ));
        let (y0, y1) = toast_rows(self.height);
        self.touch_chrome(y0, y1, Wave::Fast);
    }

    /// Whether a notice is on screen (drivers keep ticking until it clears).
    pub fn notice_showing(&self) -> bool {
        self.toast.is_some()
    }

    pub(super) fn touch(&mut self, y0: u32, y1: u32, wave: Wave) {
        self.pending_page = merge_change(self.pending_page, y0, y1, wave);
    }

    pub(super) fn touch_chrome(&mut self, y0: u32, y1: u32, wave: Wave) {
        self.change = merge_change(self.change, y0, y1, wave);
    }

    /// The current screen as an update. Only the rows that changed are marked
    /// for refresh; the rest of the buffer is still complete.
    /// Under Primitive Launcher the whole browser (bars and pages) follows the system theme (`Some(dark)`), and the
    /// Display sheet has no Day and Night. Standalone (`None`) it follows Night, as before.
    pub fn set_system_theme(&mut self, dark: Option<bool>) {
        if self.system_dark != dark {
            self.system_dark = dark;
            self.redraw_all_calm();
        }
    }

    /// Draw the whole screen again with a flash: after the system's own screens, or when the theme changed.
    pub fn redraw_all(&mut self) {
        self.change = Some(((0, self.height), Wave::Flash));
    }

    /// Draw the whole screen again without a flash: the launcher's redraw after a theme switch. The launcher has just
    /// flashed the screen (showing it in the new theme), and a second flash would show the old theme for a moment.
    pub fn redraw_all_calm(&mut self) {
        self.change = Some(((0, self.height), Wave::Text));
    }

    pub fn frame(&mut self) -> Update {
        loop {
            self.collect();
            if self.paint.is_none() {
                self.start_paint_if_needed();
                if self.paint.is_none() {
                    return self.compose_update(true);
                }
            }
            self.wait_paint();
        }
    }

    pub fn try_frame(&mut self) -> Option<Update> {
        self.collect();
        if self.paint.is_none() {
            self.start_paint_if_needed();
        }
        if self.paint.is_none() {
            return Some(self.compose_update(true));
        }
        // A page is being painted: the bars and sheets are shown now, over the picture on screen. The page change
        // stays pending until its paint is applied (`page_ready`).
        let pressed = self.pressed_rows.is_some();
        if self.change.is_none() && !pressed {
            return None;
        }
        Some(self.compose_update(false))
    }

    pub fn page_ready(&mut self) -> Option<Update> {
        self.collect();
        if self.paint.is_none() {
            self.start_paint_if_needed();
        }
        // Nothing is waiting: a finished prefetch, or a paint that a tap already applied. No screen to show.
        if self.paint.is_none()
            && self.change.is_none()
            && self.pending_page.is_none()
            && self.pressed_rows.is_none()
        {
            return None;
        }
        self.try_frame()
    }

    /// Apply the paint and the prefetch the worker has finished. A result from an older state of the page is dropped.
    pub(super) fn collect(&mut self) {
        if let Some(flight) = self.paint {
            if let Some(result) = self.host.take::<Vec<u8>>(flight.ticket) {
                self.paint = None;
                self.apply_paint(flight, result);
            }
        }
        if let Some(flight) = self.prefetch {
            if let Some(result) = self.host.take::<Vec<u8>>(flight.ticket) {
                self.prefetch = None;
                if let Some(gray) = result {
                    if flight.gen == self.page_gen && !gray.is_empty() {
                        self.cache_screen(flight.key, gray);
                    }
                }
            }
        }
    }

    /// Block until the paint in flight is applied.
    fn wait_paint(&mut self) {
        let Some(flight) = self.paint.take() else { return };
        let result = self.host.wait::<Vec<u8>>(flight.ticket);
        self.apply_paint(flight, result);
    }

    /// Show a finished paint. `None` means the paint panicked or the worker is gone: keep the picture on screen.
    fn apply_paint(&mut self, flight: Flight, result: Option<Vec<u8>>) {
        if flight.gen != self.page_gen {
            // The page changed while it was painted; the next frame paints again.
            return;
        }
        self.page_dirty = false;
        let Some(gray) = result else { return };
        self.cache_screen(flight.key, gray.clone());
        self.page_gray = gray;
        self.page_gray_offset = flight.offset;
    }

    /// Get the picture of the page for the current scroll position and chrome offset: from the cache when it is
    /// there, otherwise by a paint on the worker (which `collect` applies). Does nothing when the page is current.
    pub(super) fn start_paint_if_needed(&mut self) {
        if !self.page_dirty || self.paint.is_some() {
            return;
        }
        let want = self.page_offset();
        let key = (self.scroll_pos().round() as u64, want);
        if let Some((_, gray)) = self.page_cache.iter().find(|(k, _)| *k == key) {
            self.page_gray = gray.clone();
            self.page_gray_offset = want;
            self.page_dirty = false;
            return;
        }
        if !self.has_page() {
            self.page_gray = vec![255u8; (self.width * self.height) as usize];
            self.page_gray_offset = want;
            self.page_dirty = false;
            return;
        }
        let delay = self.host.paint_delay();
        let gen = self.page_gen;
        let ticket = self.host.post(move |slots: &mut Slots| {
            if !delay.is_zero() {
                thread::sleep(delay);
            }
            slots
                .page
                .as_mut()
                .map(|p| p.render_gray_at(want))
                .unwrap_or_default()
        });
        match ticket {
            Some(ticket) => self.paint = Some(Flight { ticket, gen, key, offset: want }),
            // The worker is gone: keep the picture on screen rather than wait for a paint that cannot come.
            None => self.page_dirty = false,
        }
    }

    /// The whole screen as it is now, and clears the pending change. `include_page` also shows the pending page
    /// change; without it only the bars and other chrome changes are shown.
    fn compose_update(&mut self, include_page: bool) -> Update {
        let page = if include_page { self.pending_page.take() } else { None };
        let merged = match (self.change.take(), page) {
            (None, None) => None,
            (Some(c), None) | (None, Some(c)) => Some(c),
            (Some((a, wa)), Some((b, wb))) => Some(((a.0.min(b.0), a.1.max(b.1)), wa.max(wb))),
        };
        // The highlight drawn by `press` is taken back here. When nothing else changed, only the key's own
        // columns go back to the panel; otherwise the highlight's rows join the change, across the full width.
        let pressed = self.pressed_rows.take();
        let (rows, wave, cols) = match (merged, pressed) {
            (None, Some((cols, rows))) => (rows, Wave::Fast, cols),
            (None, None) => ((0, self.height), Wave::Text, (0, self.width)),
            (Some((rows, wave)), None) => (rows, wave, (0, self.width)),
            (Some((rows, wave)), Some((_, (a, b)))) => {
                ((rows.0.min(a), rows.1.max(b)), wave.max(Wave::Fast), (0, self.width))
            }
        };
        let mut gray = if self.keyboard {
            self.compose_keyboard()
        } else {
            self.compose_browse()
        };
        // Inverted: near-white page backgrounds become black, and very dark levels snap to pure black so the
        // background is fully dark rather than a faint grey. Under Primitive Launcher the system theme decides;
        // standalone, Night does.
        if self.system_dark.unwrap_or(self.settings.night) {
            for px in gray.iter_mut() {
                let inverted = 255 - *px;
                *px = if inverted < 16 { 0 } else { inverted };
            }
        }
        self.last_gray = gray.clone();
        Update {
            gray,
            width: self.width,
            height: self.height,
            rows,
            cols,
            wave,
        }
    }

    /// The e-ink waveforms the reader chose: (page content, bars and taps).
    pub fn wave_names(&self) -> (String, String) {
        (self.settings.page_wave.clone(), self.settings.ui_wave.clone())
    }

    /// Whether page turns are dithered in software (Display sheet).
    pub fn page_dither(&self) -> bool {
        self.settings.page_dither
    }

    /// Whether near-black and near-white page pixels are drawn pure (Display sheet).
    pub fn solid_text(&self) -> bool {
        self.settings.solid_text
    }

    pub(super) fn emit_frame(&mut self) {
        if self.draw.is_none() {
            return;
        }
        self.collect();
        let update = self.compose_update(true);
        if let Some(draw) = self.draw.as_mut() {
            draw(update);
        }
    }

    pub(super) fn mark_page_dirty(&mut self) {
        self.page_cache.clear();
        self.page_changed();
    }

    /// The page changed: the picture on screen is out of date, and a paint started now belongs to an older state.
    pub(super) fn page_changed(&mut self) {
        self.page_dirty = true;
        self.page_gen += 1;
    }

    /// Keep a rendered screen; the cache holds the last few (a page turn needs the next one).
    pub(super) fn cache_screen(&mut self, key: (u64, u32), gray: Vec<u8>) {
        self.page_cache.retain(|(k, _)| *k != key);
        self.page_cache.push((key, gray));
        // The current screen and both neighbours, plus one.
        if self.page_cache.len() > 4 {
            self.page_cache.remove(0);
        }
    }

    pub(super) fn compose_browse(&mut self) -> Vec<u8> {
        let (w, h) = (self.width, self.height);
        // The picture of the page on screen (never painted here: `start_paint_if_needed` does that).
        let mut out = if self.page_gray.len() == (w * h) as usize {
            self.page_gray.clone()
        } else {
            vec![255u8; (w * h) as usize]
        };
        if let Some(sheet) = self.sheet {
            let layer = self.ensure_sheet(sheet).gray.clone();
            blit(
                &mut out,
                w,
                h - MENU_H + SHEET_TITLE_H,
                &layer,
                MENU_H - SHEET_TITLE_H,
            );
            let header = self.ensure_sheet_header(sheet).gray.clone();
            blit(&mut out, w, h - MENU_H, &header, SHEET_TITLE_H);
        } else if self.chrome {
            let top = self.ensure_top().gray.clone();
            blit(&mut out, w, 0, &top, TOP_H);
            let bottom = self.ensure_bottom().gray.clone();
            blit(&mut out, w, h - BOT_H, &bottom, BOT_H);
        }
        // The quick settings panel drops from the top edge over the bars and the page.
        if self.quick && !self.keyboard {
            let panel = self.ensure_quick().gray.clone();
            blit(&mut out, w, 0, &panel, QUICK_H);
        }
        if let Some((message, _)) = self.toast.clone() {
            let layer = self.toast_layer(&message);
            let (y0, _) = toast_rows(self.height);
            blit(&mut out, w, y0, &layer, 88);
        }
        if self.images_loading() {
            let s = self.net.stats();
            let done = s.deferred_done.load(std::sync::atomic::Ordering::Relaxed) as f32;
            let total = s.deferred.load(std::sync::atomic::Ordering::Relaxed).max(1) as f32;
            let y = if self.chrome { TOP_H - PROGRESS_H } else { 0 };
            progress_strip(&mut out, w, y, done / total);
        }
        out
    }

    pub(super) fn toast_layer(&mut self, message: &str) -> Vec<u8> {
        if self
            .toast_cache
            .as_ref()
            .map_or(true, |(m, _)| m != message)
        {
            let html = ui::toast(message);
            let mut page = Page::new(
                &html,
                "kb://chrome/",
                self.width,
                88,
                self.net.clone(),
                &mut || true,
            );
            let gray = page.render_gray();
            self.toast_cache = Some((message.to_string(), gray));
        }
        self.toast_cache
            .as_ref()
            .map(|(_, g)| g.clone())
            .unwrap_or_default()
    }

    pub(super) fn compose_keyboard(&mut self) -> Vec<u8> {
        let (w, h) = (self.width, self.height);
        let mut out = vec![255u8; (w * h) as usize];
        // Address bar on top (the same height as the page's bar), suggestions, then the keys.
        let bar = self.ensure_field().gray.clone();
        blit(&mut out, w, 0, &bar, TOP_H);
        let list = self.ensure_suggestions().gray.clone();
        blit(&mut out, w, TOP_H, &list, self.kb_y() - TOP_H);
        let keys = self.ensure_key(self.layer).gray.clone();
        blit(&mut out, w, self.kb_y(), &keys, h - self.kb_y());
        out
    }

    pub(super) fn render_layer(&self, html: &str, base: &str, w: u32, h: u32) -> Cached {
        let mut page = Page::new(html, base, w, h, self.net.clone(), &mut || true);
        let gray = page.render_gray();
        Cached {
            key: String::new(),
            page,
            gray,
        }
    }

    pub(super) fn ensure_top(&mut self) -> &mut Cached {
        let key = format!("{}|{}", self.url, self.can_go_back());
        if self.top.as_ref().map_or(true, |c| c.key != key) {
            let html = ui::top(&self.url);
            let mut c = self.render_layer(&html, "kb://chrome/", self.width, TOP_H);
            c.key = key;
            self.top = Some(c);
        }
        self.top.as_mut().expect("top layer")
    }

    pub(super) fn ensure_bottom(&mut self) -> &mut Cached {
        let marked = self.bookmarks.iter().any(|(u, _)| *u == self.url);
        let key = format!(
            "{}{}{}{}|{}",
            self.can_go_back() as u8,
            self.can_go_forward() as u8,
            marked as u8,
            self.reader as u8,
            self.tabs.len().max(1)
        );
        if self.bottom.as_ref().map_or(true, |c| c.key != key) {
            let html = ui::bottom(
                self.can_go_back(),
                self.can_go_forward(),
                marked,
                self.tabs.len().max(1),
            );
            let mut c = self.render_layer(&html, "kb://chrome/", self.width, BOT_H);
            c.key = key;
            self.bottom = Some(c);
        }
        self.bottom.as_mut().expect("bottom layer")
    }

    /// The quick settings panel, rendered again only when a device value changes.
    pub(super) fn ensure_quick(&mut self) -> &mut Cached {
        let on = ui::Switches {
            night: self.settings.night,
            images: self.settings.images,
        };
        let key = format!("{:?}{:?}", self.device_status, on);
        if self.quick_layer.as_ref().map_or(true, |c| c.key != key) {
            let html = ui::quick_panel(&self.device_status, on);
            let mut c = self.render_layer(&html, "kb://chrome/", self.width, QUICK_H);
            c.key = key;
            self.quick_layer = Some(c);
        }
        self.quick_layer.as_mut().expect("quick layer")
    }

    pub(super) fn show_sheet(&mut self, sheet: Sheet) {
        self.sheet = Some(sheet);
        self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
    }

    /// The menu or tab sheet, rendered once per state.
    pub(super) fn ensure_sheet(&mut self, sheet: Sheet) -> &mut Cached {
        let cache_mb = store::cache_size() / (1024 * 1024);
        let marked = self.bookmarks.iter().any(|(u, _)| *u == self.url);
        let tabs: Vec<(String, String)> = self.tab_labels();
        let key = match sheet {
            Sheet::Menu => format!(
                "menu|{}|{}|{}|{}|{}|{}|{}|{}",
                self.reader,
                cache_mb,
                marked,
                self.url,
                self.settings.night,
                self.settings.text_pct,
                self.settings.page_width,
                self.settings.images
            ),
            Sheet::Tabs => format!("tabs|{}|{:?}", self.active, tabs),
            Sheet::Display => format!(
                "display|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
                self.settings.text_pct,
                self.settings.page_width,
                self.settings.night,
                self.settings.page_wave,
                self.settings.ui_wave,
                self.settings.flash_on_load,
                self.settings.page_dither,
                self.settings.solid_text,
                self.settings.book_lines,
                self.page_is_web && !crate::storage::scripts_off(&super::loading::host_of(&self.url))
            ),
            Sheet::Choice => format!("choice|{}|{:?}", self.choice_title, self.choice_options),
        };
        if self.sheet_layer.as_ref().map_or(true, |c| c.key != key) {
            let html = match sheet {
                Sheet::Menu => ui::menu(
                    self.reader,
                    cache_mb,
                    marked,
                    &self.display_summary(),
                    self.settings.images,
                ),
                Sheet::Tabs => ui::tabs(&tabs, self.active),
                Sheet::Choice => ui::choice(&self.choice_title, &self.choice_options),
                Sheet::Display => ui::display(
                    self.settings.text_pct,
                    self.settings.page_width,
                    // Under Primitive Launcher the theme is the system's: no Day and Night here.
                    self.system_dark.is_none().then_some(self.settings.night),
                    (self.page_is_web).then(|| !crate::storage::scripts_off(&super::loading::host_of(&self.url))),
                    &self.settings.page_wave,
                    &self.settings.ui_wave,
                    self.settings.flash_on_load,
                    self.settings.page_dither,
                    self.settings.solid_text,
                    self.settings.book_lines,
                ),
            };
            let mut c =
                self.render_layer(&html, "kb://chrome/", self.width, MENU_H - SHEET_TITLE_H);
            c.key = key;
            self.sheet_layer = Some(c);
        }
        self.sheet_layer.as_mut().expect("sheet layer")
    }

    pub(super) fn ensure_sheet_header(&mut self, sheet: Sheet) -> &mut Cached {
        let title = match sheet {
            Sheet::Menu => crate::brand::menu_title(),
            Sheet::Tabs => format!(
                "{} open {}",
                self.tabs.len(),
                if self.tabs.len() == 1 { "tab" } else { "tabs" }
            ),
            Sheet::Display => "Display".to_string(),
            Sheet::Choice => self.choice_title.clone(),
        };
        if self.sheet_header.as_ref().map_or(true, |c| c.key != title) {
            // Only the menu has a control in its title strip (Exit). Other sheets close by a tap on
            // the page above them.
            let controls = if sheet == Sheet::Menu { ui::find_button() + &ui::exit_button() } else { String::new() };
            let html = ui::sheet_header(&title, &controls);
            let mut c = self.render_layer(&html, "kb://chrome/", self.width, SHEET_TITLE_H);
            c.key = title;
            self.sheet_header = Some(c);
        }
        self.sheet_header.as_mut().expect("sheet header")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_png_has_the_right_size_and_signature() {
        let gray = vec![200u8; 4 * 3];
        let png = encode_png(4, 3, &gray).expect("encode");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert!(png.len() > 33);
    }
}
