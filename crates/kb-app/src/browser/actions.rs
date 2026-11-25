//! Browser actions named by `kb:` links: the menu, the Display settings, and the toolbar.

use super::*;
use crate::LEVEL_MAX;

pub(super) fn stays_open(href: &str) -> bool {
    matches!(
        href,
        "kb:night"
            | "kb:nighton"
            | "kb:nightoff"
            | "kb:images"
            | "kb:mark"
            | "kb:save"
            | "kb:clearcache"
            | "kb:clearhistory"
            | "kb:display"
    ) || href.starts_with("kb:font:")
        || href.starts_with("kb:width:")
        || href.starts_with("kb:closetab:")
        || href.starts_with("kb:wave:")
        || href == "kb:refresh"
}

impl Browser {
    /// Turn scripts on or off for the current site, and load the page again so the change applies.
    pub(super) fn set_site_scripts(&mut self, on: bool) {
        let host = super::loading::host_of(&self.url);
        if host.is_empty() || !self.page_is_web {
            self.notify("Scripts are set per web site");
            return;
        }
        crate::storage::set_scripts_off(&host, !on);
        self.notify(if on { "Scripts are on for this site" } else { "Scripts are off for this site" });
        self.reload();
    }

    pub(super) fn chrome_action(&mut self, href: &str) {
        eprintln!(
            "[ui] action {href} (sheet={:?}, page={}, url={})",
            self.sheet,
            self.has_page(),
            self.url
        );
        // Sheet rows and tiles can link straight to a page (Bookmarks, Saved pages, Home).
        if href.starts_with("kbp://") || href.starts_with("http://") || href.starts_with("https://")
        {
            self.open(href);
            return;
        }
        if let Some(n) = href
            .strip_prefix("kb:rm:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if n < self.bookmarks.len() {
                self.bookmarks.remove(n);
                write_pairs(&data_file("bookmarks.tsv"), &self.bookmarks);
                self.refresh_internal();
            }
            return;
        }
        if let Some(n) = href
            .strip_prefix("kb:bmup:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if n > 0 && n < self.bookmarks.len() {
                self.bookmarks.swap(n - 1, n);
                write_pairs(&data_file("bookmarks.tsv"), &self.bookmarks);
                self.refresh_internal();
            }
            return;
        }
        if let Some(n) = href
            .strip_prefix("kb:bmdown:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if n + 1 < self.bookmarks.len() {
                self.bookmarks.swap(n, n + 1);
                write_pairs(&data_file("bookmarks.tsv"), &self.bookmarks);
                self.refresh_internal();
            }
            return;
        }
        if let Some(step) = href.strip_prefix("kb:font:") {
            self.step_text(if step == "+" { 1 } else { -1 });
            return;
        }
        if let Some(w) = href
            .strip_prefix("kb:width:")
            .and_then(|s| s.parse::<u32>().ok())
        {
            self.set_width(w);
            return;
        }
        if let Some(name) = href.strip_prefix("kb:wave:page:") {
            if let Some(w) = crate::settings::PAGE_WAVES.iter().find(|w| **w == name) {
                self.settings.page_wave = w.to_string();
                self.settings.save();
                self.sheet_layer = None;
                self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
            }
            return;
        }
        if let Some(name) = href.strip_prefix("kb:wave:ui:") {
            if let Some(w) = crate::settings::UI_WAVES.iter().find(|w| **w == name) {
                self.settings.ui_wave = w.to_string();
                self.settings.save();
                self.sheet_layer = None;
                self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
            }
            return;
        }
        if let Some(index) = href.strip_prefix("kb:pick:").and_then(|s| s.parse::<usize>().ok()) {
            if let Some(key) = self.choice_key.take() {
                if self.has_page() {
                    self.page_on(move |p| p.select_pick(key, index));
                    self.mark_page_dirty();
                }
            }
            self.sheet = None;
            self.touch(0, self.height, Wave::Text);
            return;
        }
        if let Some(which) = href.strip_prefix("kb:homeshow:") {
            match which {
                "bm" => self.settings.home_bookmarks_open = !self.settings.home_bookmarks_open,
                _ => self.settings.home_recent_open = !self.settings.home_recent_open,
            }
            self.settings.save();
            self.refresh_internal();
            return;
        }
        if let Some(n) = href
            .strip_prefix("kb:tab:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            self.switch_tab(n);
            return;
        }
        if let Some(n) = href
            .strip_prefix("kb:closetab:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            self.close_tab(n);
            return;
        }
        if let Some(n) = href
            .strip_prefix("kb:sugg:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if let Some((url, _, _)) = self.suggestions.get(n).cloned() {
                self.keyboard = false;
                self.open(&url);
            }
            return;
        }
        match href {
            "kb:addr" => self.enter_keyboard(),
            "kb:reload" => self.reload(),
            "kb:back" => self.back(),
            "kb:fwd" => self.forward(),
            "kb:home" => self.open("kbp://home"),
            "kb:mark" => self.toggle_bookmark(),
            "kb:menu" => self.show_sheet(Sheet::Menu),
            "kb:tabs" => self.show_sheet(Sheet::Tabs),
            "kb:newtab" => self.new_tab(),
            "kb:closemenu" => {
                self.sheet = None;
                self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
            }
            "kb:reader" => {
                self.toggle_reader();
            }
            "kb:save" => self.save_offline(),
            "kb:marks" => self.open("kbp://bookmarks"),
            "kb:saved" => self.open("kbp://saved"),
            "kb:find" => self.begin_find(),
            "kb:display" => self.show_sheet(Sheet::Display),
            "kb:quickclose" => self.close_quick(),
            "kb:wifi" => {
                let on = !self.device_status.wifi_on;
                self.device.set_wifi(on);
                self.device_status = self.device.status();
                self.quick_changed();
            }
            _ if href.starts_with("kb:bright:") || href.starts_with("kb:warm:") => {
                let (which, level) = href.trim_start_matches("kb:").split_once(':').unwrap_or(("", "0"));
                let level = level.parse::<u32>().unwrap_or(0).min(LEVEL_MAX);
                if which == "bright" {
                    self.device.set_brightness(level);
                    self.device_status.brightness = level;
                } else {
                    self.device.set_warmth(level);
                    self.device_status.warmth = level;
                }
                self.quick_changed();
            }
            "kb:refresh" => {
                // One full flash of the whole screen: clears the ghosting of the previous pages.
                self.touch(0, self.height, Wave::Flash);
            }
            "kb:wave:dither:on" | "kb:wave:dither:off" => {
                self.settings.page_dither = href.ends_with(":on");
                self.settings.save();
                self.sheet_layer = None;
                self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
            }
            "kb:wave:lines:on" | "kb:wave:lines:off" => {
                self.settings.book_lines = href.ends_with(":on");
                self.settings.save();
                self.sheet_layer = None;
                // The page CSS changes: lay the page out again.
                self.reflow();
            }
            "kb:wave:solid:on" | "kb:wave:solid:off" => {
                self.settings.solid_text = href.ends_with(":on");
                self.settings.save();
                self.sheet_layer = None;
                self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
            }
            "kb:wave:flash:on" | "kb:wave:flash:off" => {
                self.settings.flash_on_load = href.ends_with(":on");
                self.settings.save();
                self.sheet_layer = None;
                self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
            }
            "kb:images" => {
                self.settings.images = !self.settings.images;
                self.settings.save();
                // Rebuild the page: turning images on fetches them, turning them off drops them.
                self.reflow();
                // The menu shows the new state; a notice would cover it.
                if self.sheet.is_none() {
                    self.notify(if self.settings.images {
                        "Images on"
                    } else {
                        "Images off"
                    });
                }
            }
            "kb:night" => {
                self.settings.night = !self.settings.night;
                self.set_night(self.settings.night);
            }
            "kb:nighton" => self.set_night(true),
            "kb:scripts:on" => self.set_site_scripts(true),
            "kb:scripts:off" => self.set_site_scripts(false),
            "kb:nightoff" => self.set_night(false),
            "kb:clearhistory" => {
                self.visits.clear();
                write_pairs(&data_file("history.tsv"), &self.visits);
                self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
                self.notify("History cleared");
            }
            "kb:clearcache" => {
                store::cache_clear();
                self.touch_chrome(self.height - MENU_H, self.height, Wave::Text);
                self.sheet_layer = None;
                self.notify("Cache cleared");
            }
            "kb:exit" => {
                kb_net::save_cookies();
                self.save_session();
                self.exit = true;
            }
            _ => {}
        }
    }

    /// Move one step along `TEXT_STEPS` (-1 smaller, +1 larger) and reflow the page.
    pub(super) fn step_text(&mut self, dir: i32) {
        let cur = TEXT_STEPS
            .iter()
            .position(|&p| p == self.settings.text_pct)
            .unwrap_or(2) as i32;
        let next = (cur + dir).clamp(0, TEXT_STEPS.len() as i32 - 1) as usize;
        if next as i32 == cur {
            return;
        }
        self.settings.text_pct = TEXT_STEPS[next];
        self.settings.save();
        self.reflow();
    }

    /// Choose the page width (one of `PAGE_WIDTHS`) and reflow the page.
    pub(super) fn set_width(&mut self, width: u32) {
        if !PAGE_WIDTHS.contains(&width) || width == self.settings.page_width {
            return;
        }
        self.settings.page_width = width;
        self.settings.save();
        self.reflow();
    }

    /// Short label for the menu row: "Medium, 100%".
    pub(super) fn display_summary(&self) -> String {
        format!(
            "{}, {}%",
            width_name(self.settings.page_width),
            self.settings.text_pct
        )
    }

    /// Open or close the quick settings panel (a swipe down from the top edge).
    pub fn quick_settings(&mut self) {
        if self.quick {
            self.close_quick();
        } else if !self.keyboard {
            self.device_status = self.device.status();
            self.quick = true;
            self.quick_layer = None;
            self.touch(0, self.height, Wave::Text);
        }
    }

    pub(super) fn close_quick(&mut self) {
        self.quick = false;
        self.touch(0, self.height, Wave::Text);
    }

    /// The panel changed a value: redraw it.
    fn quick_changed(&mut self) {
        self.quick_layer = None;
        self.touch_chrome(0, QUICK_H, Wave::Text);
    }

    pub(super) fn set_night(&mut self, on: bool) {
        self.settings.night = on;
        self.settings.save();
        self.touch(0, self.height, Wave::Flash);
        if self.sheet.is_none() {
            self.notify(if on {
                "Night mode on"
            } else {
                "Night mode off"
            });
        }
    }
}
