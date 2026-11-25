//! Browser state and UI: navigation, history, bookmarks, saved pages, reader
//! mode, and the browser's own interface (address bar, bottom toolbar, menu,
//! QWERTY keyboard with caret, home screen). The chrome is HTML rendered by the
//! same engine as web pages, and every rendered layer is cached so that a tap
//! only re-renders what changed.
//!
//! Link kinds: `kb:<action>` runs a browser action; `kbp://…` is an internal
//! page; http(s) is a normal navigation.

mod addr;
mod archive;
mod brand;
mod device;
mod browser;
mod html;
mod layout;
mod settings;
mod storage;
mod ui;

pub use addr::normalize;
pub use brand::NAME as BROWSER_NAME;
pub use device::{DeviceControl, DeviceStatus, NoDevice, LEVEL_MAX};
pub use primitive_quick::Stats;
pub use browser::{encode_png, Browser, Cancelled, Update, Wave};
pub use layout::{kb_top, BOT_H, MENU_H, PROGRESS_H, QUICK_H, SHEET_TITLE_H, TOP_H};
pub use settings::{width_name, PAGE_WIDTHS};
pub(crate) use addr::split_url;
pub(crate) use browser::{keyboard_rows, Key, Layer};
pub(crate) use html::esc;
