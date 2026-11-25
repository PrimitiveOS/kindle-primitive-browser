//! The browser: state and its public API. Behaviour is split by responsibility across the
//! submodules, each holding part of `impl Browser`.

use std::{
    fs,
    panic::AssertUnwindSafe,
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

use kb_engine::Page;
use kb_net::{store, Policy, SyncNet};

use crate::addr::*;
use crate::device::{DeviceControl, DeviceStatus, NoDevice};
use crate::html::*;
use crate::layout::*;
use crate::settings::*;
use crate::storage::*;

mod actions;
mod find;
mod driver;
mod frame;
mod host;
mod input;
mod internal;
mod keyboard;
mod library;
mod lines;
mod loading;
mod nav;
mod session;
mod tabs;
mod types;

use actions::stays_open;
use host::{ChromeSlots, Host, Slots};
use types::{Cached, Flight, PreviousPage, Sheet, TabSave};

pub use frame::encode_png;
pub(crate) use keyboard::{keyboard_rows, Key, Layer};
pub use types::{Cancelled, Update, Wave};

pub struct Browser {
    width: u32,
    height: u32,
    net: Arc<SyncNet>,
    /// The page worker: every page document lives there (see host.rs and docs/06-page-worker.md).
    host: Host,

    history: Vec<String>,
    scrolls: Vec<f64>,
    idx: usize,
    /// The page picture on screen (the last one painted). It stays on screen while a new one is painted.
    page_gray: Vec<u8>,
    /// Vertical offset the cached page image was painted with.
    page_gray_offset: u32,
    /// The page changed since its picture was painted.
    page_dirty: bool,
    /// Bumped by every change to the page. A paint from an older generation is dropped.
    page_gen: u64,
    /// A change that touches the page is waiting; it is shown when the page's paint finishes.
    pending_page: Option<((u32, u32), Wave)>,
    /// The paint of the page on screen, and the prefetch of the next screen, sent to the worker.
    paint: Option<Flight>,
    prefetch: Option<Flight>,
    source: String,
    reader_source: Option<String>,
    page_is_web: bool,
    url: String,
    reader: bool,
    chrome: bool,
    sheet: Option<Sheet>,
    /// Tabs other than the active one (the active slot holds an empty placeholder).
    tabs: Vec<TabSave>,
    active: usize,
    /// Key of the text field being edited from a page, when the keyboard is in input mode.
    input_target: Option<u64>,
    settings: Settings,
    /// The page left most recently, kept live so Back is instant. Capped at one: memory stays at two pages.
    previous: Option<PreviousPage>,
    /// URL of the page on screen (to tell it apart from the previous one).
    page_url: String,
    /// Rendered screens of the page, by (scroll position, chrome offset). Cleared when the content changes;
    /// filled while idle with the screen after the current one, so a page turn copies it instead of painting.
    page_cache: Vec<((u64, u32), Vec<u8>)>,
    toast: Option<(String, Instant)>,
    toast_cache: Option<Cached>,

    keyboard: bool,
    typed: Vec<char>,
    caret: usize,
    /// The whole address is selected (set when the address bar opens on a page): the next
    /// key replaces it, and the suggestions show the bookmarks and recent pages.
    select_all: bool,
    layer: Layer,
    /// Address suggestions: (url, title, is_bookmark). Bookmarks are listed first.
    suggestions: Vec<(String, String, bool)>,
    field: Option<Cached>,
    suggest: Option<Cached>,
    keys: [Option<Cached>; 4],
    /// The field being edited shows stars (a page password field).
    masked: bool,
    /// Entries that pushState created: the document they belong to, and the state pushed with them.
    /// Going back to one of these on the same document fires popstate instead of loading the page again.
    pushed_states: std::collections::HashMap<String, (String, String)>,
    /// The find keyboard is open. The last query, its matches (device pixels) and the match shown.
    find_entry: bool,
    find_query: String,
    find_matches: Vec<f64>,
    find_index: usize,
    /// The select being chosen from: its title, its options (label, selected) and its node key.
    choice_title: String,
    choice_options: Vec<(String, bool)>,
    choice_key: Option<u64>,

    top: Option<Cached>,
    bottom: Option<Cached>,
    sheet_layer: Option<Cached>,
    sheet_header: Option<Cached>,
    /// The chrome worker: the bars, sheets, keyboard parts and notice are drawn there (see `post_layer`).
    chrome_host: Host<ChromeSlots>,
    /// The download or page build in progress (see loading.rs). The UI thread handles input meanwhile.
    pending: Option<loading::Pending>,
    /// Wakes the UI when the worker or the download thread has finished a job.
    waker: Option<host::Waker>,
    /// The progress strip's fraction while a load runs.
    progress: Option<f32>,
    /// Chrome renders sent to the chrome worker and not yet applied, in order: (ticket, slot).
    chrome_flights: std::collections::VecDeque<(u64, usize)>,

    exit: bool,
    navigating_history: bool,
    bookmarks: Vec<(String, String)>,
    visits: Vec<(String, String)>,
    /// Pending screen change: union of changed rows and the strongest waveform.
    change: Option<((u32, u32), Wave)>,
    /// Under Primitive Launcher the bars follow the system theme (`Some(dark)`); standalone (`None`) they follow Night.
    system_dark: Option<bool>,
    images_seen: usize,
    last_image_refresh: Instant,
    /// Last composed screen, used to draw the tap highlight without re-rendering.
    last_gray: Vec<u8>,
    /// Rows of the highlight shown by `press`, to be restored on the next frame.
    /// The tap highlight on screen: its columns and rows, taken back by the next frame.
    pressed_rows: Option<((u32, u32), (u32, u32))>,
    /// Receives a frame as soon as a navigation starts, so the address bar shows the new URL while the page loads.
    draw: Option<Box<dyn FnMut(Update)>>,
    /// The quick settings panel (dropped from the top edge) is open.
    quick: bool,
    quick_layer: Option<Cached>,
    /// The device's own controls, and the values the panel last showed.
    device: Box<dyn DeviceControl>,
    device_status: DeviceStatus,
    /// An address to load in place of this page (a script navigation or a meta refresh).
    pending_nav: Option<String>,
    /// How many redirects are in progress, so a redirect loop stops after a few hops.
    redirect_depth: u8,
}

impl Browser {
    pub fn new(width: u32, height: u32) -> Browser {
        ensure_defaults();
        let net = Arc::new(SyncNet::new(SUBRESOURCE_MAX_BYTES));
        let chrome_host = Host::chrome(net.clone());
        Browser {
            width,
            height,
            host: Host::new(net.clone()),
            net,
            history: Vec::new(),
            scrolls: Vec::new(),
            idx: 0,
            page_gray: Vec::new(),
            page_gray_offset: u32::MAX,
            page_dirty: true,
            page_gen: 0,
            pending_page: None,
            paint: None,
            prefetch: None,
            source: String::new(),
            reader_source: None,
            page_is_web: false,
            url: String::new(),
            reader: false,
            chrome: true,
            sheet: None,
            tabs: vec![TabSave::empty()],
            active: 0,
            input_target: None,
            settings: Settings::load(),
            previous: None,
            page_url: String::new(),
            page_cache: Vec::new(),
            toast: None,
            toast_cache: None,
            keyboard: false,
            typed: Vec::new(),
            caret: 0,
            select_all: false,
            layer: Layer::Lower,
            suggestions: Vec::new(),
            field: None,
            suggest: None,
            keys: [None, None, None, None],
            masked: false,
            pushed_states: std::collections::HashMap::new(),
            find_entry: false,
            find_query: String::new(),
            find_matches: Vec::new(),
            find_index: 0,
            choice_title: String::new(),
            choice_options: Vec::new(),
            choice_key: None,
            top: None,
            bottom: None,
            sheet_layer: None,
            sheet_header: None,
            chrome_host,
            pending: None,
            waker: None,
            progress: None,
            chrome_flights: std::collections::VecDeque::new(),
            exit: false,
            navigating_history: false,
            bookmarks: read_pairs(&data_file("bookmarks.tsv")),
            visits: read_pairs(&data_file("history.tsv")),
            change: None,
            system_dark: None,
            images_seen: 0,
            last_image_refresh: Instant::now(),
            last_gray: Vec::new(),
            pressed_rows: None,
            draw: None,
            quick: false,
            quick_layer: None,
            device: Box::new(NoDevice),
            device_status: DeviceStatus::unavailable(),
            pending_nav: None,
            redirect_depth: 0,
        }
    }

    /// Install the sink that shows a frame during a load (the driver puts it on the panel).
    pub fn set_draw(&mut self, draw: Box<dyn FnMut(Update)>) {
        self.draw = Some(draw);
    }

    /// Install the device's controls (the Kindle driver does this); the panel then shows its values.
    pub fn set_device(&mut self, mut device: Box<dyn DeviceControl>) {
        self.device_status = device.status();
        self.device = device;
    }

    /// Install the callback the page worker calls when a job finishes. The device sends its input loop a
    /// wake-up with it, so the finished page is shown without waiting for an input event.
    pub fn set_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        self.host.set_waker(waker.clone());
        self.chrome_host.set_waker(waker.clone());
        self.waker = Some(waker);
    }

    /// Whether a page paint is running on the worker.
    pub fn paint_in_flight(&self) -> bool {
        self.paint.is_some()
    }

    /// Whether a page is open (the worker holds it).
    pub(super) fn has_page(&self) -> bool {
        self.host.mirror().has_page
    }

    /// Run `f` on the page on screen, on the worker, and return its result. `None` when no page is open.
    /// Waits behind a paint that is already running; chrome never calls this.
    pub(super) fn page_on<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Page) -> R + Send + 'static,
    ) -> Option<R> {
        if !self.has_page() {
            return None;
        }
        self.host.ask(move |slots| slots.page.as_mut().map(f)).flatten()
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn can_go_back(&self) -> bool {
        self.idx > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.idx + 1 < self.history.len()
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn exit_requested(&self) -> bool {
        self.exit
    }

    pub fn title(&self) -> String {
        self.host.mirror().title
    }

    pub fn content_height(&self) -> f64 {
        self.host.mirror().content_height
    }

    pub fn scroll_pos(&self) -> f64 {
        self.host.mirror().scroll_y
    }

    /// Images still being fetched in the background.
    pub fn images_loading(&self) -> bool {
        let s = self.net.stats();
        s.deferred.load(std::sync::atomic::Ordering::Relaxed)
            > s.deferred_done.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod responsive_tests {
    use super::*;

    /// The browser's data folder is set through the environment, which all tests share: the browser tests run one at
    /// a time, so one test's visits do not change another's home page.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn browser() -> Browser {
        let data = std::env::temp_dir().join(format!("kb-app-tests-{}", std::process::id()));
        std::env::set_var("KB_DATA", &data);
        std::env::set_var(
            "KB_FONTS",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/NotoSans-Regular.ttf"),
        );
        let mut b = Browser::new(600, 800);
        b.open("kbp://home");
        let _ = b.frame();
        b
    }

    /// A paint of the page is running on the worker, held by a test delay, so the test can act meanwhile.
    fn slow_browser() -> Browser {
        let b = browser();
        b.host.set_paint_delay(Duration::from_millis(1500));
        b
    }

    #[test]
    fn the_bars_are_shown_while_a_page_is_painted() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut b = slow_browser();
        b.page_step(1.0);
        assert!(b.try_frame().is_none(), "a page change waits for its paint");
        assert!(b.paint_in_flight());
        b.action("menu");
        let bars = b.try_frame().expect("the menu is shown while the page is painted");
        assert!(b.paint_in_flight(), "the menu came before the paint finished");
        assert_eq!(bars.rows.1, b.height, "the update is the menu's rows");
        let _ = b.frame();
        assert!(!b.paint_in_flight());
    }

    #[test]
    fn the_address_bar_opens_while_a_page_is_painted() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The bars are on screen; the page is changed (its cache cleared), so it must be painted again.
        let mut b = browser();
        b.host.set_paint_delay(Duration::from_millis(1500));
        b.mark_page_dirty();
        assert!(b.try_frame().is_none(), "the page change waits for its paint");
        assert!(b.paint_in_flight());
        b.tap(300.0, 20.0);
        let keyboard = b.try_frame().expect("the keyboard opens while the page is painted");
        assert!(b.paint_in_flight(), "the keyboard came before the paint finished");
        assert_eq!(keyboard.rows.1, b.height);
        let _ = b.frame();
        assert!(!b.paint_in_flight());
    }

    #[test]
    fn a_sheet_is_drawn_without_holding_the_screen() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The menu's drawing takes a second on the chrome worker. Opening the menu must return at once, and the
        // screen must not wait for the drawing: the bars and the sheet's place are shown, and the sheet follows.
        let mut b = browser();
        b.chrome_host.set_paint_delay(Duration::from_millis(1000));
        let started = Instant::now();
        b.action("menu");
        let shown = b.try_frame().expect("the menu's place is shown while it is drawn");
        assert!(started.elapsed() < Duration::from_millis(500), "the UI thread did not wait for the drawing");
        assert_eq!(shown.rows.1, b.height);
        let _ = b.frame();
        assert!(b.chrome_flights.is_empty(), "the menu is drawn once frame() returns");
    }

    #[test]
    fn a_slow_download_does_not_hold_the_screen() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // A local server that answers after 1.5 s. Opening the address returns at once; the menu opens during the
        // download; frame() then waits for the page, which is shown with its title.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                std::thread::sleep(Duration::from_millis(1500));
                let body = "<!doctype html><html><head><title>Slow</title></head><body><p>Hello slow page</p></body></html>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
            }
        });
        let mut b = browser();
        let started = Instant::now();
        b.open(&format!("http://127.0.0.1:{port}/slow"));
        assert!(started.elapsed() < Duration::from_millis(500), "open returns while the page downloads");
        assert!(b.pending.is_some(), "the download is pending");
        b.action("menu");
        let shown = b.try_frame().expect("the menu is shown during the download");
        assert!(started.elapsed() < Duration::from_millis(1000), "the menu did not wait for the download");
        assert_eq!(shown.rows.1, b.height);
        let _ = b.frame();
        assert!(b.pending.is_none(), "the page has been shown");
        assert_eq!(b.title(), "Slow");
    }

    #[test]
    fn a_paint_the_page_has_outrun_is_not_shown() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Two turns while the first paint runs: the first picture is out of date and must not be shown.
        let mut turned = slow_browser();
        turned.page_step(1.0);
        assert!(turned.try_frame().is_none());
        turned.page_step(1.0);
        let shown = turned.frame();

        let mut straight = browser();
        straight.page_step(1.0);
        straight.page_step(1.0);
        let expected = straight.frame();
        assert_eq!(shown.gray, expected.gray);
        assert_eq!(turned.scroll_pos(), straight.scroll_pos());
    }
}

