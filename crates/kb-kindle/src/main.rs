//! Kindle driver. Reads touch and page-turn buttons from evdev, drives the
//! browser, and shows each frame on the panel: in-process through the framebuffer (`eink`), or with
//! the device's FBInk when that path cannot draw the screen as it is.
//!
//! Files (all under /mnt/us/browser):
//!   start.txt  optional start URL (default: Hacker News)
//!   /var/tmp/kb-frame.bmp  last frame, for debugging (RAM, not kept across restarts)
//!   log         written by the launcher (stderr)

use std::{
    collections::VecDeque,
    cell::RefCell,
    fs::{self, File},
    io::Read,
    os::unix::io::AsRawFd,
    panic::{self, AssertUnwindSafe},
    process::Command,
    rc::Rc,
    sync::Arc,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{channel, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

mod device;
mod eink;
mod profile;

use kb_app::{Browser, Update, Wave, TOP_H};

const DIR: &str = "/mnt/us/browser";
const FBINK: &str = "/mnt/us/browser/bin/fbink";
/// The panel's native frame: upright portrait. The touch controller always reports in this frame,
/// whichever way the Kindle is turned, so touches are turned into the screen's frame here.
const NATIVE_W: u32 = profile::ACTIVE.screen_w;
const NATIVE_H: u32 = profile::ACTIVE.screen_h;

/// The turn of the Kindle: 0 upright, 2 upside down (the left-handed portrait). The Kindle has no
/// landscape, so the browser always draws the portrait screen.
static PANEL_ROTA: AtomicU32 = AtomicU32::new(0);

/// Turn a point from the panel's native frame into the screen's frame for the turn. This is the same
/// turn the picture gets, so a finger lands on what it touches in every orientation.
fn turn_point(rota: u32, x: f32, y: f32) -> (f32, f32) {
    if rota == 2 {
        (NATIVE_W as f32 - 1.0 - x, NATIVE_H as f32 - 1.0 - y)
    } else {
        (x, y)
    }
}

/// Whether the page-turn buttons swap their directions for this turn. They are on the top and bottom
/// edges: upside down, the button on the reader's "next" side moves, so forward and back trade places.
fn page_buttons_swapped(rota: u32) -> bool {
    rota == 2
}
/// Full-screen flash every N partial refreshes, to clear e-ink ghosting.

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0x00;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const KEY_PAGEUP: u16 = profile::ACTIVE.page_back_key;
const KEY_PAGEDOWN: u16 = profile::ACTIVE.page_forward_key;

/// The turn the framebuffer is drawn in, from FBInk's state (`fbink -e` prints `currentRota=N`): 0 upright,
/// 2 upside down. The screen's own direction, so the controls follow what the reader sees, not the
/// physical tilt. Landscape (1 and 3) is not supported and gives no turn.
fn turn_from_fbink(state: &str) -> Option<u32> {
    let value = state
        .split([';', '\n'])
        .find_map(|pair| pair.trim().strip_prefix("currentRota="))?
        .trim()
        .parse::<u32>()
        .ok()?;
    match value {
        0 => Some(0),
        2 => Some(2),
        _ => None,
    }
}

/// The turn the framework shows: its accelerometer property (`U` upright, `D` upside down, `R` and `L`
/// landscape, which is not supported and gives no turn). This is the orientation the reader sees.
fn turn_from_framework(value: &str) -> Option<u32> {
    match value.trim() {
        "U" => Some(0),
        "D" => Some(2),
        _ => None,
    }
}

fn framework_turn() -> Option<u32> {
    let out = Command::new("lipc-get-prop").args(["com.lab126.winmgr", "accelerometer"]).output().ok()?;
    turn_from_framework(&String::from_utf8_lossy(&out.stdout))
}

/// FBInk's framebuffer rotation, kept beside the framework's value in the log.
fn framebuffer_turn() -> Option<u32> {
    let out = Command::new(FBINK).arg("-e").output().ok()?;
    turn_from_fbink(&String::from_utf8_lossy(&out.stdout))
}

/// The launcher's turn ("U" or "D" in /var/tmp/kl/orientation), used when neither source above has one.
/// Written by Primitive Launcher, which keeps the framework stopped.
fn launcher_turn() -> Option<u32> {
    turn_from_framework(&std::fs::read_to_string("/var/tmp/kl/orientation").ok()?)
}

/// The turn now: the framework's, else the framebuffer's. Both are returned for the log.
fn display_turn() -> (Option<u32>, Option<u32>) {
    (framework_turn(), framebuffer_turn())
}

/// Follow the screen's turn: once a second, and each change sets the turn that touch and the page
/// buttons use. The log records both sources, so a disagreement with the text on screen shows.
fn display_turn_watch() {
    loop {
        thread::sleep(Duration::from_secs(1));
        let (framework, framebuffer) = display_turn();
        let Some(turn) = framework.or(framebuffer).or_else(launcher_turn) else { continue };
        if PANEL_ROTA.swap(turn, Ordering::Relaxed) != turn {
            log(&format!("display turned: rota {turn} (framework {framework:?}, framebuffer {framebuffer:?})"));
        }
    }
}

/// `_IOR('E', 0x40 + code, struct input_absinfo)` for the 24-byte absinfo.
const fn eviocgabs(code: u16) -> libc::c_ulong {
    (0x8000_0000u32 | (24 << 16) | (0x45 << 8) | (0x40 + code as u32)) as libc::c_ulong
}

#[repr(C)]
#[derive(Default)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

/// Matches the kernel's `struct input_event`. The Kindle kernel is 32-bit, so
/// its timeval fields are 32-bit regardless of what the libc crate's time_t is.
#[repr(C)]
struct InputEvent {
    tv_sec: i32,
    tv_usec: i32,
    kind: u16,
    code: u16,
    value: i32,
}

#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<InputEvent>() == 16);

#[derive(Debug)]
enum Ev {
    /// A swipe down that starts at the top edge: quick settings.
    QuickSettings,
    Tap(f32, f32),
    LongTap(f32, f32),
    TabStep(i32),
    Scroll(f64),
    Page(f64),
    Back,
    Forward,
    /// The page worker finished a job: a paint or a prefetch. The page on screen can change now.
    PageReady,
}

/// A touch that starts this close to the top edge (device pixels) is an edge swipe.
const EDGE_H: f32 = 40.0;

/// Run by Primitive Launcher (`PRIMITIVE_LAUNCHER=1`): the launcher is the system layer. Its quick settings, sleep and
/// theme are the system's, so the browser leaves the top-edge swipe to it and follows the system theme.
static UNDER_LAUNCHER: AtomicBool = AtomicBool::new(false);
/// Set by SIGUSR1: the launcher asks for the whole screen again (after a wake, or a theme change).
static REDRAW: AtomicBool = AtomicBool::new(false);

extern "C" fn on_redraw(_: libc::c_int) {
    REDRAW.store(true, Ordering::Relaxed);
}

/// The system theme under Primitive Launcher (`dark_mode` in its settings), or `None` standalone.
fn system_dark() -> Option<bool> {
    if !UNDER_LAUNCHER.load(Ordering::Relaxed) {
        return None;
    }
    let dir = std::env::var("KL_DIR").unwrap_or_else(|_| "/mnt/us/launcher".into());
    let text = fs::read_to_string(format!("{dir}/settings.txt")).unwrap_or_default();
    Some(text.lines().any(|l| l.trim() == "dark_mode=on"))
}

fn log(msg: &str) {
    eprintln!("[kb] {msg}");
}

/// Find an input device by its kernel name (e.g. "cyttsp5_mt", "gpio-keys").
fn find_input(name_hint: &str) -> Option<String> {
    for entry in fs::read_dir("/sys/class/input").ok()?.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if !fname.starts_with("event") {
            continue;
        }
        let name = fs::read_to_string(entry.path().join("device/name")).unwrap_or_default();
        if name.trim().contains(name_hint) {
            return Some(format!("/dev/input/{fname}"));
        }
    }
    None
}

fn abs_range(file: &File, code: u16) -> Option<AbsInfo> {
    let mut info = AbsInfo::default();
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), eviocgabs(code) as _, &mut info as *mut AbsInfo) };
    (rc == 0).then_some(info)
}

/// The axis range of a panel `size` pixels across, from 0 to size - 1.
fn native_range(size: u32) -> AbsInfo {
    AbsInfo { minimum: 0, maximum: size as i32 - 1, ..AbsInfo::default() }
}

/// A test input stream named in a file of the browser folder (`test-touch`, `test-buttons`): a FIFO that carries
/// recorded `input_event` records in place of the device's own input node. Absent on the device by default, so the
/// browser reads its real touch and buttons. Used by the automated device tests (docs/07-device-tests.md).
fn test_input(name: &str) -> Option<String> {
    fs::read_to_string(format!("{DIR}/{name}"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn scale(raw: i32, info: &AbsInfo, out: u32) -> f32 {
    let span = (info.maximum - info.minimum).max(1) as f32;
    ((raw - info.minimum) as f32 / span * (out as f32 - 1.0)).clamp(0.0, out as f32 - 1.0)
}

/// Touch reader: taps, vertical drags (scroll), horizontal swipes (back/forward).
fn touch_loop(path: String, tx: Sender<Ev>, test: bool) {
    let mut file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => return log(&format!("cannot open {path}: {e}")),
    };
    let (rx_info, ry_info) = if test {
        // A test stream (a FIFO) has no axis ranges: the panel's own range (touch is 1:1 with the native pixels).
        (Some(native_range(NATIVE_W)), Some(native_range(NATIVE_H)))
    } else {
        (abs_range(&file, ABS_MT_POSITION_X), abs_range(&file, ABS_MT_POSITION_Y))
    };
    log(&format!("touch {path}{}: x={:?} y={:?}", if test { " (test input)" } else { "" }, rx_info.as_ref().map(|i| (i.minimum, i.maximum)), ry_info.as_ref().map(|i| (i.minimum, i.maximum))));
    let (Some(xi), Some(yi)) = (rx_info, ry_info) else {
        return log("touch ranges unavailable");
    };

    let (mut raw_x, mut raw_y) = (0i32, 0i32);
    let mut active = false;
    let mut start: Option<(f32, f32)> = None;
    let mut last: (f32, f32) = (0.0, 0.0);
    let mut down_at = Instant::now();

    loop {
        let ev = match read_event(&mut file) {
            Some(e) => e,
            None => return log("touch read failed"),
        };
        match (ev.kind, ev.code) {
            (EV_ABS, ABS_MT_TRACKING_ID) => {
                if ev.value >= 0 {
                    active = true;
                    start = None;
                    down_at = Instant::now();

                } else if active {
                    active = false;
                    let end = last;
                    if let Some((sx, sy)) = start {
                        let (dx, dy) = (end.0 - sx, end.1 - sy);
                        let held = down_at.elapsed();
                        // A finger never lands perfectly still: allow a generous slop before
                        // a touch counts as a swipe. Taps and long presses are told apart by
                        // how long the finger stayed down.
                        let still = dx.abs() < 40.0 && dy.abs() < 40.0;
                        let on_bar = sy < TOP_H as f32;
                        let ev = if still && held < Duration::from_millis(800) {
                            Some(Ev::Tap(end.0, end.1))
                        } else if still {
                            Some(Ev::LongTap(end.0, end.1))
                        } else if sy < EDGE_H && dy > 120.0 && dy.abs() > dx.abs() * 1.5 {
                            // A swipe down from the very top edge opens quick settings: the browser's own, or under
                            // Primitive Launcher the system's (the launcher sees the same swipe).
                            if UNDER_LAUNCHER.load(Ordering::Relaxed) { None } else { Some(Ev::QuickSettings) }
                        } else if on_bar && dx.abs() > 180.0 && dx.abs() > dy.abs() * 1.5 {
                            // Swipe along the address bar moves between tabs.
                            Some(Ev::TabStep(if dx > 0.0 { -1 } else { 1 }))
                        } else if dy.abs() > dx.abs() * 1.2 && dy.abs() > 80.0 {
                            Some(Ev::Scroll(-(dy as f64)))
                        } else if dx.abs() > dy.abs() * 1.5 && dx.abs() > 180.0 {
                            Some(if dx > 0.0 { Ev::Back } else { Ev::Forward })
                        } else {
                            None
                        };
                        if let Some(e) = ev {
                            let _ = tx.send(e);
                        }
                    }
                }
            }
            (EV_ABS, ABS_MT_POSITION_X) => raw_x = ev.value,
            (EV_ABS, ABS_MT_POSITION_Y) => raw_y = ev.value,
            (EV_SYN, SYN_REPORT) => {
                if active {
                    let rota = PANEL_ROTA.load(Ordering::Relaxed);
                    let (nx, ny) = (scale(raw_x, &xi, NATIVE_W), scale(raw_y, &yi, NATIVE_H));
                    last = turn_point(rota, nx, ny);
                    if start.is_none() {
                        start = Some(last);
                        log(&format!(
                            "touch down raw=({raw_x},{raw_y}) native=({nx:.0},{ny:.0}) rota={rota} screen=({:.0},{:.0})",
                            last.0, last.1
                        ));
                    }
                }
            }
            _ => {}
        }
    }
}

/// How long a page-turn press waits: if the other page button goes down within it, the two are one chord (both held
/// to restart the launcher) and no page turns.
const CHORD_WINDOW: Duration = Duration::from_millis(120);

/// Page-turn buttons (gpio-keys). Press only; repeats ignored. A press turns the page after `CHORD_WINDOW`, unless the
/// other page button goes down first.
fn button_loop(path: String, tx: Sender<Ev>, test: bool) {
    let mut file = match File::open(&path) {
        Ok(f) => f,
        Err(e) => return log(&format!("cannot open {path}: {e}")),
    };
    log(&format!("buttons {path} open{}", if test { " (test input)" } else { "" }));
    // Each press or chord change gets a new number; a scheduled turn sends only if its number is still the latest.
    let generation = Arc::new(AtomicU64::new(0));
    // Index 0 is the forward button (KEY_PAGEDOWN), 1 the back button (KEY_PAGEUP).
    let mut held = [false; 2];
    loop {
        let Some(ev) = read_event(&mut file) else {
            return log("button read failed");
        };
        if ev.kind != EV_KEY {
            continue;
        }
        log(&format!("key code={} value={}", ev.code, ev.value));
        let index = match ev.code {
            KEY_PAGEDOWN => 0,
            KEY_PAGEUP => 1,
            _ => continue,
        };
        if ev.value == 0 {
            held[index] = false;
            continue;
        }
        if ev.value != 1 {
            continue;
        }
        held[index] = true;
        let number = generation.fetch_add(1, Ordering::SeqCst) + 1;
        if held[1 - index] {
            // Both buttons down within the window: a chord, which turns no page.
            continue;
        }
        let swapped = page_buttons_swapped(PANEL_ROTA.load(Ordering::Relaxed));
        let forward = if swapped { -1.0 } else { 1.0 };
        let turn = if index == 0 { forward } else { -forward };
        let tx = tx.clone();
        let generation = Arc::clone(&generation);
        thread::spawn(move || {
            thread::sleep(CHORD_WINDOW);
            if generation.load(Ordering::SeqCst) == number {
                let _ = tx.send(Ev::Page(turn));
            }
        });
    }
}

fn read_event(file: &mut File) -> Option<InputEvent> {
    let mut buf = [0u8; std::mem::size_of::<InputEvent>()];
    file.read_exact(&mut buf).ok()?;
    // Safe: InputEvent is repr(C) plain data and buf has the same size.
    Some(unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const InputEvent) })
}

/// Drawing to the panel. Regions are cropped to the rows that changed, and the
/// waveform follows the kind of change (fast for chrome, text quality for pages).
struct Display {
    /// When the last update was sent to the panel.
    last_draw: Instant,
    /// Draw in-process (true) or always with fbink (false); from the tuning file.
    direct: bool,
    /// The e-ink waveforms chosen in the browser's Display sheet.
    page_wave: String,
    ui_wave: String,
    /// Software dithering for page turns, from the Display sheet.
    page_dither: bool,
    /// Near-black and near-white page pixels drawn pure, from the Display sheet.
    solid_text: bool,
    /// The framebuffer for in-process drawing; `None` when it cannot be opened (fbink is used).
    panel: Option<eink::Panel>,
}

impl Display {
    fn show(&mut self, u: &Update) {
        let (y0, y1) = u.rows;
        if y1 <= y0 {
            return;
        }
        let t0 = Instant::now();
        let flash = u.wave == Wave::Flash;
        // A full flash is always GC16. Otherwise the reader's own choice: page content and bars/taps.
        let wave = if flash {
            "GC16"
        } else {
            match u.wave {
                Wave::Fast => self.ui_wave.as_str(),
                _ => self.page_wave.as_str(),
            }
        };
        // Only the changed columns and rows go to the panel (a key highlight is one key, not the row).
        let (x, cw, mut region) = u.changed_area();
        // Dithering is for page content only: bars, progress and flashes keep their own rendering.
        let dither = self.page_dither && !flash && u.wave == Wave::Text;
        // Page content is cut to the panel's 16 gray steps before it is sent. Each extra gray level is one more
        // transition for the driver to count, and too many of them trigger its flash pass. KB_QUANTIZE=0 turns it off.
        if u.wave == Wave::Text && dither {
            dither16(&mut region, u.width, y0);
        } else if u.wave == Wave::Text && quantize_pages() {
            quantize16(&mut region);
        }
        // Letters in a site's dark gray (or its Night inverse) flash on every turn: the panel reaches a gray by
        // swinging through the opposite extreme. Pure black and white letters change directly, as in the stock reader.
        if u.wave == Wave::Text && self.solid_text {
            solidify(&mut region);
        }
        let wave = wave.to_string();
        let path = self.blit(x, cw, y0, y1 - y0, &region, &wave, flash);
        self.last_draw = Instant::now();
        log(&format!("eink {wave} rows ({y0}, {y1}) {path} in {:?}", t0.elapsed()));
    }

    /// Draw in-process when possible, else with fbink: the screen is turned, or the driver refused the update.
    /// Returns which path drew.
    fn blit(&mut self, x: u32, width: u32, y: u32, rows: u32, gray: &[u8], wave: &str, flash: bool) -> &'static str {
        if self.direct {
            if let (Some(panel), Some(code)) = (self.panel.as_mut(), eink::wave_code(wave)) {
                match panel.draw(x, width, y, rows, gray, code, flash) {
                    Ok(true) => return "direct",
                    Ok(false) => {}
                    Err(e) => log(&format!("panel: {e}; using fbink")),
                }
            }
        }
        blit_region(x, width, y, rows, gray, wave, flash);
        "fbink"
    }
}

/// Settings for measuring on the device, read from `tuning.txt` in the browser folder before each event, so
/// they can be changed while the browser runs. Lines are `key=value`; a missing file or key keeps the default.
///
/// - `prefetch_delay_ms` (450): how long after a panel update the next screens may be rendered.
/// - `direct` (1): draw in-process; 0 draws every update with fbink.
struct Tuning {
    prefetch_delay: Duration,
    direct: bool,
    /// The file's modification time when it was last read.
    seen: Option<std::time::SystemTime>,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { prefetch_delay: Duration::from_millis(450), direct: true, seen: None }
    }
}

impl Tuning {
    fn reload(&mut self) {
        let path = format!("{DIR}/tuning.txt");
        let modified = fs::metadata(&path).and_then(|m| m.modified()).ok();
        if modified == self.seen {
            return;
        }
        self.seen = modified;
        let text = fs::read_to_string(&path).unwrap_or_default();
        *self = Tuning { seen: modified, ..Tuning::parse(&text) };
        log(&format!("tuning: prefetch_delay {:?}, direct {}", self.prefetch_delay, self.direct));
    }

    fn parse(text: &str) -> Tuning {
        let mut t = Tuning::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match (key.trim(), value.trim()) {
                ("prefetch_delay_ms", v) => {
                    if let Ok(ms) = v.parse::<u64>() {
                        t.prefetch_delay = Duration::from_millis(ms);
                    }
                }
                ("direct", v) => t.direct = v != "0",
                _ => {}
            }
        }
        t
    }
}

/// The framebuffer for in-process drawing, or `None` (logged) when it cannot be used.
fn open_panel() -> Option<eink::Panel> {
    match eink::Panel::open() {
        Ok(p) => {
            log("panel: drawing in-process");
            Some(p)
        }
        Err(e) => {
            log(&format!("panel: {e}; using fbink"));
            None
        }
    }
}

/// Whether page content is cut to 16 gray steps before it is sent (on unless KB_QUANTIZE=0).
fn quantize_pages() -> bool {
    std::env::var("KB_QUANTIZE").map_or(true, |v| v != "0")
}

/// Round each gray to the nearest of the 16 panel steps (0, 17, 34, ... 255).
fn quantize16(gray: &mut [u8]) {
    for px in gray.iter_mut() {
        *px = ((*px as u32 + 8) / 17 * 17).min(255) as u8;
    }
}

/// The 8x8 ordered-dither thresholds (0..63), the Bayer matrix.
const BAYER8: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// Ordered dither to the panel's 16 gray steps (multiples of 17), in place. Each pixel lies between two steps;
/// the Bayer pattern decides which one it takes, so the average gray is kept in every area. The pattern is
/// tied to the screen position, so the same gray gives the same dots on every turn. Done in the browser, not
/// by FBInk: the FBInk build on the Kindle cannot dither images.
fn dither16(gray: &mut [u8], width: u32, y0: u32) {
    let w = width as usize;
    for (i, px) in gray.iter_mut().enumerate() {
        let x = i % w;
        let y = y0 as usize + i / w;
        let base = (*px as u32) / 17;
        let frac = (*px as u32) % 17;
        // Round up when the fraction is above the pixel's threshold: frac/17 > (t + 0.5) / 64.
        let t = BAYER8[y % 8][x % 8] as u32;
        let up = frac * 128 > 17 * (2 * t + 1);
        *px = ((base + u32::from(up)) * 17).min(255) as u8;
    }
}

/// Draw the two panel steps next to black as black and the two next to white as white (0, 17, 34 -> 0 and
/// 221, 238, 255 -> 255). Text set in #111 or #222 then has pure letter bodies; the anti-aliased edges keep
/// their grays.
fn solidify(gray: &mut [u8]) {
    for px in gray.iter_mut() {
        if *px <= 34 {
            *px = 0;
        } else if *px >= 221 {
            *px = 255;
        }
    }
}

/// The frame file fbink reads. It lives in RAM (/var is tmpfs), so a page turn does not write to the
/// storage partition. An 8-bit grayscale BMP needs no compression, which makes it much faster to produce
/// than a PNG; fbink reads it directly.
const FRAME_FILE: &str = "/var/tmp/kb-frame.bmp";

/// Show `rows` rows of gray pixels at row `y` with the given fbink waveform.
fn blit_region(x: u32, width: u32, y: u32, rows: u32, gray: &[u8], wave: &str, flash: bool) {
    let bmp = gray_bmp(width, rows, gray);
    let path = FRAME_FILE;
    if let Err(e) = fs::write(path, bmp) {
        return log(&format!("write frame: {e}"));
    }
    let mut cmd = Command::new(FBINK);
    cmd.arg("-g").arg(format!("file={path},x={x},y={y}"));
    cmd.args(["-W", wave]);
    if flash {
        cmd.arg("-f");
    }
    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(s) => log(&format!("fbink exit {s}")),
        Err(e) => log(&format!("fbink: {e}")),
    }
}

/// An 8-bit grayscale BMP of `width` x `rows` pixels. `gray` holds the rows top to bottom, one byte per pixel.
/// The rows are stored bottom-up, as BMP files usually are, with each row padded to four bytes.
fn gray_bmp(width: u32, rows: u32, gray: &[u8]) -> Vec<u8> {
    let stride = (width as usize + 3) & !3;
    let pixels = stride * rows as usize;
    let palette = 256 * 4;
    let offset = 14 + 40 + palette;
    let size = offset + pixels;
    let mut out = Vec::with_capacity(size);
    // File header.
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    // Info header (BITMAPINFOHEADER).
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(rows as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]); // no compression, sizes and resolution left at zero
    // Grey palette.
    for v in 0..=255u8 {
        out.extend_from_slice(&[v, v, v, 0]);
    }
    // Pixels, bottom row first.
    let row_len = width as usize;
    let padding = [0u8; 3];
    for row in (0..rows as usize).rev() {
        let start = row * row_len;
        let line = gray.get(start..start + row_len).unwrap_or(&[]);
        out.extend_from_slice(line);
        out.extend_from_slice(&padding[..stride - line.len()]);
    }
    out
}

/// State shared by the input loop and the browser's progress pump.
struct Shared {
    rx: Receiver<Ev>,
    pending: VecDeque<Ev>,
    display: Display,
}

/// Resident memory of this process in kB, from /proc (0 where /proc is absent).
fn rss_kb() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

/// Writes a fatal signal's number and fault address to stderr (the browser log) before the process ends.
/// Only `write(2)` is used here, which is safe inside a signal handler.
extern "C" fn on_fatal(sig: libc::c_int, info: *mut libc::siginfo_t, _ctx: *mut libc::c_void) {
    fn put(bytes: &[u8]) {
        unsafe { libc::write(2, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
    }
    fn hex(mut value: usize, out: &mut [u8; 18]) -> &[u8] {
        let digits = b"0123456789abcdef";
        let mut at = out.len();
        loop {
            at -= 1;
            out[at] = digits[value & 15];
            value >>= 4;
            if value == 0 {
                break;
            }
        }
        out[at - 2] = b'0';
        out[at - 1] = b'x';
        &out[at - 2..]
    }
    let addr = if info.is_null() { 0 } else { unsafe { (*info).si_addr() as usize } };
    let mut buf = [0u8; 18];
    put(b"\n[crash] fatal signal ");
    put(&[b'0' + (sig as u8 / 10), b'0' + (sig as u8 % 10)]);
    put(b" at address ");
    put(hex(addr, &mut buf));
    put(b"\n");
    // Restore the default action and raise it again, so the process ends as it would have.
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

fn install_crash_log() {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_fatal as *const () as usize;
        action.sa_flags = libc::SA_SIGINFO;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGBUS, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGSEGV, &action, std::ptr::null_mut());
    }
}

fn main() {
    install_crash_log();
    log("start");
    if std::env::var("PRIMITIVE_LAUNCHER").as_deref() == Ok("1") {
        UNDER_LAUNCHER.store(true, Ordering::Relaxed);
        // SAFETY: the handler only stores to an atomic, which is safe in a signal handler.
        unsafe { libc::signal(libc::SIGUSR1, on_redraw as *const () as libc::sighandler_t) };
        log("under Primitive Launcher: the system layer owns quick settings, sleep and the theme");
    }
    let start = fs::read_to_string(format!("{DIR}/start.txt"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "kbp://home".into());

    let (tx, rx) = channel();
    let touch_test = test_input("test-touch");
    if let Some(path) = touch_test.clone().or_else(|| find_input(profile::ACTIVE.touch_name)) {
        let t = tx.clone();
        let test = touch_test.is_some();
        thread::spawn(move || touch_loop(path, t, test));
    } else {
        log("no touch device found");
    }
    let buttons_test = test_input("test-buttons");
    if let Some(path) = buttons_test.clone().or_else(|| find_input("gpio-keys")) {
        let t = tx.clone();
        let test = buttons_test.is_some();
        thread::spawn(move || button_loop(path, t, test));
    } else {
        log("no page-button device found");
    }

    let shared = Rc::new(RefCell::new(Shared {
        rx,
        pending: VecDeque::new(),
        display: Display { last_draw: Instant::now(), direct: true, page_wave: "GC16".into(), ui_wave: "DU".into(), page_dither: false, solid_text: true, panel: open_panel() },
    }));

    // While a page loads, the progress strip is drawn and any tap cancels the wait,
    // so the input is handled right away instead of after the page finishes.
    let (framework, framebuffer) = display_turn();
    let turn = framework.or(framebuffer).unwrap_or(0);
    PANEL_ROTA.store(turn, Ordering::Relaxed);
    log(&format!("turn at start: rota {turn} (framework {framework:?}, framebuffer {framebuffer:?})"));
    thread::spawn(display_turn_watch);
    let mut b = Browser::new(NATIVE_W, NATIVE_H);
    // The page worker wakes the input loop when a paint or prefetch is finished.
    let wake_tx = tx.clone();
    b.set_waker(Arc::new(move || {
        let _ = wake_tx.send(Ev::PageReady);
    }));
    b.set_system_theme(system_dark());
    // A navigation paints the address bar at once (the browser calls this before fetching).
    let draw_state = shared.clone();
    b.set_draw(Box::new(move |u| draw_state.borrow_mut().display.show(&u)));

    // The Kindle's own frontlight, warmth, battery and Wi-Fi for the quick settings panel.
    b.set_device(Box::new(device::KindleDevice));

    let t0 = Instant::now();
    // Resumes the tabs saved when the browser last left; `start` only when there are none.
    b.resume(&start);
    // The page arrives later (a download, then the build) and wakes the loop (`PageReady`).
    if let Some(u) = b.try_frame() {
        shared.borrow_mut().display.show(&u);
    }
    log(&format!("started loading in {:?}: {}", t0.elapsed(), b.url()));

    // How long to wait for input before an idle tick. Just after an event it is short, so the next screen is
    // prefetched soon after the panel has drawn this one rather than when the reader may already be pressing
    // the button again.
    const IDLE: Duration = Duration::from_millis(400);
    const SOON: Duration = Duration::from_millis(30);
    let mut idle = IDLE;
    let mut tuning = Tuning::default();
    loop {
        if REDRAW.swap(false, Ordering::Relaxed) {
            log("redraw requested by the launcher");
            b.set_system_theme(system_dark());
            b.redraw_all_calm();
            if let Some(u) = b.try_frame() {
                shared.borrow_mut().display.show(&u);
            }
        }
        tuning.reload();
        shared.borrow_mut().display.direct = tuning.direct;
        // The waveforms the reader chose in its Display sheet (they can change between events).
        {
            let (page, ui) = b.wave_names();
            let mut s = shared.borrow_mut();
            s.display.page_wave = page;
            s.display.ui_wave = ui;
            s.display.page_dither = b.page_dither();
            s.display.solid_text = b.solid_text();
        }
        let next = shared.borrow_mut().pending.pop_front();
        let ev = match next {
            Some(ev) => ev,
            None => {
                let received = shared.borrow().rx.recv_timeout(idle);
                match received {
                    Ok(ev) => ev,
                    Err(RecvTimeoutError::Timeout) => {
                        // No rendering while the panel is still drawing the last update: the prefetch keeps the
                        // processor and memory busy for 300-650 ms, which is longer than a page turn on the panel.
                        let since = shared.borrow().display.last_draw.elapsed();
                        if since < tuning.prefetch_delay {
                            idle = tuning.prefetch_delay - since;
                            continue;
                        }
                        // Idle: prefetch the next screens, bring in finished images, clear notices whose time is up.
                        let t_tick = Instant::now();
                        if let Some(u) = b.tick() {
                            shared.borrow_mut().display.show(&u);
                        }
                        if t_tick.elapsed() > Duration::from_millis(50) {
                            // It did work (a prefetch); check again soon for the other neighbour.
                            idle = SOON;
                        } else {
                            idle = IDLE;
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        };
        // A redraw the launcher asked for while the browser was frozen comes before the event that waited behind
        // the freeze, so that event is never drawn in the old theme first.
        if REDRAW.swap(false, Ordering::Relaxed) {
            log("redraw requested by the launcher");
            b.set_system_theme(system_dark());
            b.redraw_all_calm();
        }
        if let Ev::PageReady = ev {
            // The page worker finished a paint: show the page if it was waiting for one.
            if let Some(u) = b.page_ready() {
                log(&format!("page ready, rows {:?}, wave {:?}", u.rows, u.wave));
                shared.borrow_mut().display.show(&u);
            }
            idle = SOON;
            continue;
        }
        let t_action = Instant::now();
        idle = SOON;
        log(&format!("event {ev:?}"));
        // Tap feedback first: the element turns black before the action runs.
        if let Ev::Tap(x, y) | Ev::LongTap(x, y) = ev {
            if let Some(u) = b.press(x, y) {
                shared.borrow_mut().display.show(&u);
            }
        }
        // A panic in page code must not take the whole browser down.
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            match ev {
                Ev::Tap(x, y) => b.tap(x, y),
                Ev::LongTap(x, y) => b.long_press(x, y),
                Ev::QuickSettings => b.quick_settings(),
                Ev::TabStep(dir) => b.tab_step(dir),
                Ev::Scroll(dy) => b.scroll(dy),
                Ev::Page(d) => b.page_step(d),
                Ev::Back => b.back(),
                Ev::Forward => b.forward(),
                // Handled before the match: a finished paint is not an action.
                Ev::PageReady => {}
            }
            let action_ms = t_action.elapsed();
            let t_frame = Instant::now();
            // The bars and menus show now; a page change waits for its paint (`PageReady`).
            let update = b.try_frame();
            if let Some(update) = &update {
                log(&format!(
                    "action {action_ms:?}, frame {:?}, rows {:?}, wave {:?}, rss {} kB",
                    t_frame.elapsed(),
                    update.rows,
                    update.wave,
                    rss_kb()
                ));
            } else {
                log(&format!("action {action_ms:?}, page paint in flight, rss {} kB", rss_kb()));
            }
            update
        }));
        match outcome {
            Ok(Some(update)) => shared.borrow_mut().display.show(&update),
            Ok(None) => {}
            Err(_) => log("recovered from a panic while handling an event"),
        }
        if b.exit_requested() {
            log("exit requested");
            // A plain clear, no flash: the framework draws the library right after, which saves the flash time.
            let _ = Command::new(FBINK).args(["-c"]).status();
            // Leave without dropping the browser. On the Oasis 3 the drop crashed with SIGBUS ("[crash] fatal
            // signal 07" in the log); the launcher then restarted the browser, so Exit took two taps. Everything
            // that matters was saved by kb:exit, and the kernel closes the open descriptors on exit.
            log("bye");
            std::process::exit(0);
        }
    }
    // Shutdown in steps, so a crash in the last one shows which step it was.
    log("closing: dropping the browser");
    drop(b);
    log("closing: browser dropped");
    drop(shared);
    log("bye");
}

#[cfg(test)]
mod quantize_tests {
    use super::*;

    #[test]
    fn quantize_rounds_to_the_sixteen_steps() {
        let mut px = vec![0u8, 8, 9, 16, 17, 100, 128, 254, 255];
        quantize16(&mut px);
        assert_eq!(px, vec![0, 0, 17, 17, 17, 102, 136, 255, 255]);
    }

    #[test]
    fn tuning_reads_keys_and_keeps_defaults() {
        let t = Tuning::parse("prefetch_delay_ms=0\ndirect=0\n");
        assert_eq!(t.prefetch_delay, Duration::ZERO);
        assert!(!t.direct);
        let d = Tuning::parse("junk\nprefetch_delay_ms=abc\n");
        assert_eq!(d.prefetch_delay, Duration::from_millis(450));
        assert!(d.direct);
    }

    #[test]
    fn dither_keeps_the_average_gray_and_uses_panel_steps() {
        // A flat 8 (between 0 and 17) over a whole 8x8 cell: about 8 on average, only 0 or 17 per pixel.
        let mut cell = vec![8u8; 64];
        dither16(&mut cell, 8, 0);
        assert!(cell.iter().all(|&p| p == 0 || p == 17));
        let mean = cell.iter().map(|&p| p as u32).sum::<u32>() as f64 / 64.0;
        assert!((mean - 8.0).abs() < 1.0, "mean {mean}");
        // A step exactly on a panel level stays put.
        let mut steps = vec![0u8, 17, 34, 255];
        dither16(&mut steps, 4, 0);
        assert_eq!(steps, vec![0, 17, 34, 255]);
    }

    #[test]
    fn solid_text_makes_near_black_and_near_white_pure() {
        // #111 text (17) and its Night inverse (238) become pure; mid grays (anti-aliasing) stay.
        let mut px = vec![0u8, 17, 34, 51, 136, 204, 221, 238, 255];
        solidify(&mut px);
        assert_eq!(px, vec![0, 0, 0, 51, 136, 204, 255, 255, 255]);
    }
}

#[cfg(test)]
mod bmp_tests {
    use super::*;

    #[test]
    fn gray_bmp_has_the_right_size_and_rows_bottom_up() {
        // 3 pixels wide: each row is padded to 4 bytes. Row 0 (top) is 1 2 3, row 1 is 4 5 6.
        let bmp = gray_bmp(3, 2, &[1, 2, 3, 4, 5, 6]);
        assert_eq!(&bmp[0..2], b"BM");
        let offset = u32::from_le_bytes(bmp[10..14].try_into().unwrap()) as usize;
        assert_eq!(offset, 14 + 40 + 1024);
        assert_eq!(bmp.len(), offset + 8);
        assert_eq!(u32::from_le_bytes(bmp[2..6].try_into().unwrap()) as usize, bmp.len());
        assert_eq!(&bmp[offset..offset + 4], &[4, 5, 6, 0]);
        assert_eq!(&bmp[offset + 4..offset + 8], &[1, 2, 3, 0]);
        assert_eq!(u16::from_le_bytes(bmp[28..30].try_into().unwrap()), 8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upside_down_turns_the_touch_frame_and_upright_does_not() {
        let (w, h) = (NATIVE_W as f32 - 1.0, NATIVE_H as f32 - 1.0);
        assert_eq!(turn_point(0, 0.0, 0.0), (0.0, 0.0));
        // Left-handed portrait: the top-left of the panel is the bottom-right of the screen.
        assert_eq!(turn_point(2, 0.0, 0.0), (w, h));
        assert_eq!(turn_point(2, w, h), (0.0, 0.0));
    }

    #[test]
    fn page_buttons_trade_places_only_upside_down() {
        assert!(!page_buttons_swapped(0));
        assert!(page_buttons_swapped(2));
    }

    #[test]
    fn the_framework_turn_is_upright_or_upside_down_and_landscape_gives_none() {
        assert_eq!(turn_from_framework("U\n"), Some(0));
        assert_eq!(turn_from_framework("D"), Some(2));
        assert_eq!(turn_from_framework("R"), None);
        assert_eq!(turn_from_framework("L"), None);
    }

    #[test]
    fn the_framebuffer_turn_is_upright_or_upside_down_and_landscape_gives_none() {
        assert_eq!(turn_from_fbink("FBINK_VERSION='1.x';viewWidth=1264;currentRota=0;viewHeight=1680"), Some(0));
        assert_eq!(turn_from_fbink("viewWidth=1264;currentRota=2;viewHeight=1680\n"), Some(2));
        assert_eq!(turn_from_fbink("currentRota=1;viewWidth=1680"), None);
        assert_eq!(turn_from_fbink("currentRota=3"), None);
        assert_eq!(turn_from_fbink("viewWidth=1264"), None);
    }
}
