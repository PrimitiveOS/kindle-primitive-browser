//! Input: taps, long presses, scrolling, the press highlight, and hit testing for the bars,
//! sheets, keyboard and page.

use super::*;

impl Browser {
    pub fn scroll(&mut self, dy: f64) {
        // A swipe while the quick settings panel is open dismisses it, as on the Kindle.
        if self.quick && !self.keyboard {
            if dy.abs() > 8.0 {
                self.close_quick();
            }
            return;
        }
        if self.keyboard {
            // The suggestion list scrolls; the address bar and the keys stay put.
            let cached = self.ensure_suggestions();
            cached.page.scroll_by(dy);
            cached.gray = cached.page.render_gray_at(0);
            self.touch_chrome(TOP_H, self.kb_y(), Wave::Text);
            return;
        }
        // An open sheet (menu, tabs) takes the swipe and scrolls its own list.
        if let Some(sheet) = self.sheet {
            let cached = self.ensure_sheet(sheet);
            cached.page.scroll_by(dy);
            cached.gray = cached.page.render_gray_at(0);
            self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
            return;
        }
        // Scrolling down hides the bars to give the page room; scrolling up brings them back.
        if dy > 8.0 && self.chrome {
            self.set_chrome(false);
        } else if dy < -8.0 && !self.chrome {
            self.set_chrome(true);
        }
        if self.has_page() {
            self.page_on(move |p| p.scroll_by(dy));
            self.page_changed();
            self.touch(0, self.height, Wave::Text);
        }
    }

    /// Turn one screen (`dir` = +1 down, -1 up), as a book reader does: no overlap, and the text lines of
    /// the new screen on the rows of the old one (see `lines`). A turn forward hides the bars; a turn back
    /// leaves them as they are (they would move the page by their height, off the line rows).
    pub fn page_step(&mut self, dir: f64) {
        if self.quick || self.keyboard || self.sheet.is_some() || !self.has_page() {
            // The panel, keyboard or sheet takes it as a swipe.
            self.scroll((self.height - TOP_H) as f64 * 0.9 * dir);
            return;
        }
        let step = self.turn_distance(dir);
        if dir > 0.0 && self.chrome {
            self.set_chrome(false);
        }
        if self.has_page() {
            self.page_on(move |p| p.scroll_by(step));
            self.page_changed();
            self.touch(0, self.height, Wave::Text);
        }
    }

    pub(super) fn turn_distance(&self, dir: f64) -> f64 {
        let top = self.page_offset() as f64;
        let bottom = if self.chrome { self.height - BOT_H } else { self.height } as f64;
        // Forward hides the bars; back keeps them as they are.
        let (next_top, next_bottom) = if dir > 0.0 { (0.0, self.height as f64) } else { (top, bottom) };
        let plain = if dir > 0.0 { bottom - top } else { -(next_bottom - next_top) };
        let Some((scroll, max_scroll, text)) =
            self.page_on(|p| (p.scroll_y(), p.max_scroll(), p.text_lines()))
        else {
            return plain;
        };
        let turn = lines::Turn {
            scroll,
            max_scroll,
            top,
            bottom,
            next_top,
            next_bottom,
        };
        match lines::turn_to(&text, &turn, dir) {
            Some(to) => to - turn.scroll,
            None => plain,
        }
    }

    /// Long press on a link opens it in a new tab.
    pub fn long_press(&mut self, x: f32, y: f32) {
        if self.keyboard
            || self.sheet.is_some()
            || y < TOP_H as f32
            || y >= (self.height - BOT_H) as f32
        {
            return;
        }
        let off = self.page_offset() as f32;
        let href = self.page_on(move |p| p.link_at(x, y - off)).flatten();
        match href {
            Some(href) if !href.starts_with("kb:") => {
                self.new_tab();
                self.open(&href);
                self.notify("Opened in new tab");
            }
            _ => {}
        }
    }

    /// Step to the next (+1) or previous (-1) tab, wrapping around.
    pub fn tab_step(&mut self, dir: i32) {
        let n = self.tabs.len().max(1) as i32;
        if n <= 1 {
            return;
        }
        let next = (self.active as i32 + dir).rem_euclid(n) as usize;
        self.switch_tab(next);
        self.notify(&format!("Tab {} of {}", next + 1, n));
    }

    /// Run a browser action by name, e.g. `mark`, `menu`, `reader`, `home`.
    pub fn action(&mut self, name: &str) {
        self.chrome_action(&format!("kb:{name}"));
    }

    /// Handle a tap at device pixel (x, y).
    pub fn tap(&mut self, x: f32, y: f32) {
        let h = self.height as f32;
        if self.quick && !self.keyboard {
            // Links inside the panel act (brightness, warmth, Wi-Fi, close); a tap below it closes it.
            if y < QUICK_H as f32 {
                if let Some(href) = self.ensure_quick().page.link_at(x, y) {
                    self.chrome_action(&href);
                }
            } else {
                self.close_quick();
            }
            return;
        }
        if self.keyboard {
            if y < TOP_H as f32 {
                let field_box = self.ensure_field().page.link_box(x, y);
                if let Some(href) = self.ensure_field().page.link_at(x, y) {
                    if let Some(n) = href
                        .strip_prefix("kb:pos:")
                        .and_then(|v| v.parse::<usize>().ok())
                    {
                        // Tapping a character puts the caret before it, or after it on the right half.
                        let right = field_box.map_or(false, |(bx, _, bw, _)| x > bx + bw / 2.0);
                        self.caret = (n + usize::from(right)).min(self.typed.len());
                        self.select_all = false;
                        self.touch_chrome(0, TOP_H, Wave::Text);
                    } else {
                        self.keyboard_action(&href);
                    }
                }
            } else if y < self.kb_y() as f32 {
                // The suggestion list: a tap on a row opens it.
                if let Some(href) = self.ensure_suggestions().page.link_at(x, y - TOP_H as f32) {
                    self.keyboard_action(&href);
                }
            } else if let Some(href) = {
                let kb = self.kb_y() as f32;
                let layer = self.layer;
                self.ensure_key(layer).page.link_at(x, y - kb)
            } {
                self.keyboard_action(&href);
            }
            return;
        }
        if let Some(sheet) = self.sheet {
            if y >= h - MENU_H as f32 {
                let sy = y - (h - MENU_H as f32);
                let href = if sy < SHEET_TITLE_H as f32 {
                    self.ensure_sheet_header(sheet).page.link_at(x, sy)
                } else {
                    self.ensure_sheet(sheet)
                        .page
                        .link_at(x, sy - SHEET_TITLE_H as f32)
                };
                eprintln!("[ui] sheet={sheet:?} tap ({x:.0},{y:.0}) -> {href:?}");
                if let Some(href) = href {
                    if !stays_open(&href) {
                        self.sheet = None;
                    }
                    self.chrome_action(&href);
                }
                self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
            } else {
                eprintln!("[ui] sheet={sheet:?} dismissed by tap above sheet at y={y:.0}");
                self.sheet = None;
                self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
            }
            return;
        }
        if self.chrome && y < TOP_H as f32 {
            if let Some(href) = self.ensure_top().page.link_at(x, y) {
                self.chrome_action(&href);
            }
            return;
        }
        if self.chrome && y >= (h - BOT_H as f32) {
            if let Some(href) = self.ensure_bottom().page.link_at(x, y - (h - BOT_H as f32)) {
                self.chrome_action(&href);
            }
            return;
        }
        // Page content: taps map back from the screen to the page's own coordinates.
        if !self.has_page() {
            return;
        }
        let y = y - self.page_offset() as f32;
        let Some(outcome) = self.page_on(move |p| p.click_at(x, y)) else {
            return;
        };
        if outcome == kb_engine::ClickKind::Prevented {
            self.mark_page_dirty();
            self.touch(0, self.height, Wave::Text);
            return;
        }
        // A click handler may have set location.href: that address replaces the page.
        if self.follow_script_navigation() {
            return;
        }
        // Checkboxes and radio buttons change state on tap.
        if self.page_on(move |p| p.toggle_choice_at(x, y)).unwrap_or(false) {
            self.mark_page_dirty();
            self.touch(0, self.height, Wave::Text);
            return;
        }
        // A select opens its options in a sheet.
        if let Some(select) = self.page_on(move |p| p.select_at(x, y)).flatten() {
            self.choice_title = select.title;
            self.choice_options = select.options;
            self.choice_key = Some(select.key);
            self.show_sheet(Sheet::Choice);
            return;
        }
        // Text fields open the keyboard for editing; submit buttons send their form.
        let field = self.page_on(move |p| p.input_at(x, y)).flatten();
        if let Some(field) = field {
            self.input_target = Some(field.key);
            self.masked = field.password;
            self.keyboard = true;
            self.sheet = None;
            self.typed = field.value.chars().collect();
            self.caret = self.typed.len();
            self.layer = Layer::Lower;
            self.refresh_suggestions();
            self.touch_chrome(0, self.height, Wave::Text);
            return;
        }
        if let Some(submit) = self.page_on(move |p| p.form_submit_at(x, y)).flatten() {
            // A script may cancel the submit event; then the form is not sent.
            if self.page_on(move |p| p.form_submit_allowed(x, y)).unwrap_or(true) {
                self.submit(submit);
            } else {
                self.mark_page_dirty();
                self.touch(0, self.height, Wave::Text);
            }
            return;
        }
        let href = self.page_on(move |p| p.link_at(x, y)).flatten();
        match href {
            Some(href) if href.starts_with("kb:") => self.chrome_action(&href),
            Some(href) => self.open(&href),
            None => {
                // Tapping empty page toggles the bars.
                let show = !self.chrome;
                self.set_chrome(show);
            }
        }
        if outcome != kb_engine::ClickKind::None {
            self.mark_page_dirty();
            self.touch(0, self.height, Wave::Text);
        }
    }

    pub(super) fn set_chrome(&mut self, visible: bool) {
        if self.chrome != visible {
            self.chrome = visible;
            // The page moves below the address bar when the bars are shown, so repaint it at text quality.
            // The page itself is unchanged: the cached screens are keyed by offset and stay valid, so the
            // cache is kept (clearing it here threw away the prefetched screens at every change of direction).
            self.page_changed();
            self.touch(0, self.height, Wave::Text);
        }
    }

    /// Vertical offset of the page: below the address bar while the bars are shown.
    pub(super) fn page_offset(&self) -> u32 {
        if self.chrome {
            TOP_H
        } else {
            0
        }
    }

    /// Tap feedback: the element under (x, y) turns black at once, so the press
    /// registers before the action finishes. Returns the highlighted frame, or
    /// `None` when the point is not on a tappable element.
    pub fn press(&mut self, x: f32, y: f32) -> Option<Update> {
        let (bx, by, bw, bh) = self.tappable_box(x, y)?;
        if self.last_gray.is_empty() {
            return None;
        }
        let w = self.width;
        let x0 = bx.max(0.0) as u32;
        let x1 = ((bx + bw) as u32).min(w);
        let y0 = by.max(0.0) as u32;
        let y1 = ((by + bh) as u32).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let mut gray = self.last_gray.clone();
        for row in y0..y1 {
            let base = (row * w) as usize;
            for px in &mut gray[base + x0 as usize..base + x1 as usize] {
                *px = 255 - *px;
            }
        }
        self.pressed_rows = Some(((x0, x1), (y0, y1)));
        Some(Update {
            gray,
            width: w,
            height: self.height,
            rows: (y0, y1),
            cols: (x0, x1),
            wave: Wave::Fast,
        })
    }

    /// Screen box of the link under a tap, following the same layers as `tap`.
    pub(super) fn tappable_box(&mut self, x: f32, y: f32) -> Option<(f32, f32, f32, f32)> {
        let h = self.height as f32;
        if self.keyboard {
            if y < TOP_H as f32 {
                return self.ensure_field().page.link_box(x, y);
            }
            if y < self.kb_y() as f32 {
                let (bx, by, bw, bh) = self
                    .ensure_suggestions()
                    .page
                    .link_box(x, y - TOP_H as f32)?;
                return Some((bx, by + TOP_H as f32, bw, bh));
            }
            let kb = self.kb_y() as f32;
            let layer = self.layer;
            let (bx, by, bw, bh) = self.ensure_key(layer).page.link_box(x, y - kb)?;
            return Some((bx, by + kb, bw, bh));
        }
        if let Some(sheet) = self.sheet {
            if y >= h - MENU_H as f32 {
                let top = h - MENU_H as f32;
                if y < top + SHEET_TITLE_H as f32 {
                    let (bx, by, bw, bh) =
                        self.ensure_sheet_header(sheet).page.link_box(x, y - top)?;
                    return Some((bx, by + top, bw, bh));
                }
                let body_top = top + SHEET_TITLE_H as f32;
                let (bx, by, bw, bh) = self.ensure_sheet(sheet).page.link_box(x, y - body_top)?;
                return Some((bx, by + body_top, bw, bh));
            }
            return None;
        }
        if self.chrome && y < TOP_H as f32 {
            return self.ensure_top().page.link_box(x, y);
        }
        if self.chrome && y >= h - BOT_H as f32 {
            let top = h - BOT_H as f32;
            let (bx, by, bw, bh) = self.ensure_bottom().page.link_box(x, y - top)?;
            return Some((bx, by + top, bw, bh));
        }
        let off = self.page_offset() as f32;
        self.page_on(move |p| p.link_box(x, y - off))
            .flatten()
            .map(|(bx, by, bw, bh)| (bx, by + off, bw, bh))
    }
}
