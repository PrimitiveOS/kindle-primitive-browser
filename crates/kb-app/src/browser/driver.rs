//! Helpers for the desktop driver and the regression suite: find elements, tap them, and read the
//! state of the page on screen. The device driver does not use these.

use super::*;

impl Browser {
    /// Elements matching `selector` on the page on screen (debugging and tests).
    pub fn count_in_page(&self, selector: &str) -> usize {
        let sel = selector.to_string();
        self.page_on(move |p| p.count(&sel)).unwrap_or(0)
    }

    /// Centre of the first displayed element matching `selector` on the current screen,
    /// in screen pixels: keyboard keys and the field while the keyboard is open, a sheet
    /// while one is shown, otherwise the page. `None` if nothing matches.
    pub fn screen_center(&mut self, selector: &str) -> Option<(f32, f32)> {
        let centre = |(x, y, w, h): (f32, f32, f32, f32)| (x + w / 2.0, y + h / 2.0);
        if self.quick && !self.keyboard {
            // The panel covers the top of the screen, so only its own elements count.
            let (x, y) = centre(self.ensure_quick().element_box(selector)?);
            return Some((x, y));
        }
        if self.keyboard {
            if let Some(b) = self.ensure_field().element_box(selector) {
                return Some(centre(b));
            }
            // A suggestion counts as on screen only where the list shows it (above the keys).
            if let Some(b) = self.ensure_suggestions().element_box(selector) {
                let (x, y) = centre(b);
                let y = y + TOP_H as f32;
                return (y >= TOP_H as f32 && y < self.kb_y() as f32).then_some((x, y));
            }
            let (x, y) = centre(self.ensure_key(self.layer).element_box(selector)?);
            return Some((x, y + self.kb_y() as f32));
        }
        if let Some(sheet) = self.sheet {
            if let Some(b) = self.ensure_sheet_header(sheet).element_box(selector) {
                let (x, y) = centre(b);
                return Some((x, y + (self.height - MENU_H) as f32));
            }
            let (x, y) = centre(self.ensure_sheet(sheet).element_box(selector)?);
            return Some((x, y + (self.height - MENU_H + SHEET_TITLE_H) as f32));
        }
        let sel = selector.to_string();
        let (x, y) = centre(self.page_on(move |p| p.element_box(&sel)).flatten()?);
        Some((x, y + self.page_offset() as f32))
    }

    /// Scroll the page so the first element matching `selector` sits in the middle of the
    /// visible area, as a reader would before tapping it. No-op for keyboard and sheets.
    pub fn reveal(&mut self, selector: &str) {
        if self.keyboard || self.sheet.is_some() {
            return;
        }
        let sel = selector.to_string();
        let Some((_, y, _, h)) = self.page_on(move |p| p.element_box(&sel)).flatten() else {
            return;
        };
        let off = self.page_offset() as f32;
        let visible = (self.height - BOT_H) as f32 - off;
        let centre_on_screen = y + h / 2.0 + off;
        if centre_on_screen >= off && centre_on_screen <= (self.height - BOT_H) as f32 {
            return;
        }
        self.page_on(move |p| {
            let doc_y = y + p.scroll_y() as f32;
            let target = (doc_y + h / 2.0 - visible / 2.0).max(0.0) as f64;
            let delta = target - p.scroll_y();
            p.scroll_by(delta);
        });
        self.mark_page_dirty();
        self.touch(0, self.height, Wave::Text);
    }

    /// Tap the centre of the first element matching `selector`, scrolling it into view first.
    /// Returns false if no displayed element matches.
    pub fn tap_selector(&mut self, selector: &str) -> bool {
        self.reveal(selector);
        match self.screen_center(selector) {
            Some((x, y)) => {
                self.tap(x, y);
                true
            }
            None => false,
        }
    }

    /// Long press the centre of the first element matching `selector`.
    pub fn long_press_selector(&mut self, selector: &str) -> bool {
        self.reveal(selector);
        match self.screen_center(selector) {
            Some((x, y)) => {
                self.long_press(x, y);
                true
            }
            None => false,
        }
    }

    /// Tap a key of the on-screen keyboard by its character (`' '` for space).
    pub fn tap_key(&mut self, c: char) -> bool {
        let selector = format!("a[href=\"kb:c:{}\"]", c as u32);
        self.tap_selector(&selector)
    }

    /// Debug: what the page resolves at a screen point (link href, if any).
    pub fn link_at_screen(&self, x: f32, y: f32) -> Option<String> {
        let off = self.page_offset() as f32;
        self.page_on(move |p| p.link_at(x, y - off)).flatten()
    }

    /// Text of the first element matching `selector` on the page on screen (tests and debugging).
    pub fn text_in_page(&self, selector: &str) -> Option<String> {
        let sel = selector.to_string();
        self.page_on(move |p| p.text_of(&sel)).flatten()
    }

    /// Displayed elements matching `selector` on the page on screen (debugging and tests).
    /// 1 if the first element matching `selector` has its centre on the part of the screen the
    /// user can see (below the address bar and above the toolbar, the open sheet's body, or the
    /// keyboard's list above the keys), else 0. Scrolled-away elements count as not visible.
    pub fn visible_on_screen(&mut self, selector: &str) -> usize {
        let Some((x, y)) = self.screen_center(selector) else {
            return 0;
        };
        let h = self.height as f32;
        let (top, bottom) = if self.keyboard {
            (0.0, h)
        } else if self.quick {
            (0.0, QUICK_H as f32)
        } else if self.sheet.is_some() {
            (h - MENU_H as f32, h)
        } else {
            (self.page_offset() as f32, h - if self.chrome { BOT_H as f32 } else { 0.0 })
        };
        usize::from(x >= 0.0 && x < self.width as f32 && y >= top && y < bottom)
    }

    pub fn visible_in_page(&self, selector: &str) -> usize {
        let sel = selector.to_string();
        self.page_on(move |p| p.visible_count(&sel)).unwrap_or(0)
    }
}
