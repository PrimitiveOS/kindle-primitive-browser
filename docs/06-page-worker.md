# 6. Page worker: responsive bars while a page renders

Status: design for the change on branch `fix/page-chord`. Test levels are in
[03-test-and-validation.md](03-test-and-validation.md).

## 6.1 Problem

Everything ran on one thread. A page turn or a scroll ends in `Browser::frame()`, which paints the
page (`Page::render_gray_at`, about 300 ms on the Oasis 3). Taps on the top bar, the bottom bar,
the menu, the address bar and the keyboard queue behind that paint, so the bars look frozen while
a page renders.

## 6.2 Design

Two threads, one owner each:

- **UI thread** (`kb-kindle` main loop, desktop driver): owns `Browser`, the chrome state (bars,
  sheets, keyboard, address field, quick panel, toast, press highlight), the chrome layers (`Cached`,
  each a `kb_engine::Page` of `ui.rs` HTML), the screen composition and the display.
- **Page worker** (`browser/host.rs`): owns every page document: the current `Page`, the one-page
  `previous` cache, and all work on them (build, layout, scroll, paint, hit tests, forms, reader
  view, script navigation). `Page` is not `Send`, so each document is created and used only on this
  thread. Work is sent as closures (`FnOnce(&mut Slots) -> R + Send`); results come back as values.

Rules:

1. Chrome input never waits for a paint. A chrome change is composed and shown at once, over the
   last page picture that was shown.
2. Page changes are shown when the new picture is ready. While a paint is in flight, a change that
   touches the page is held (`touch`); a chrome-only change (`touch_chrome`) is shown at once.
3. Work on the page is serial, in the order it was sent. A synchronous query (hit test, scroll,
   element lookup) waits behind a paint that is already running, at most one paint. Pure chrome
   actions never make this call.
4. A paint result is applied only when no page change came after it (`page_gen`). Otherwise it is
   dropped and a new paint is sent.
5. Paints and prefetches are asynchronous. When the worker finishes, it calls a waker (the device
   sends an `Ev::PageReady` into the input channel), and the UI thread calls `page_ready()`.
6. A page build (`Page::new`) runs on the worker. While the UI thread waits for it, it keeps the
   progress strip and the cancel check (taps) going, as the fetch wait already did.

## 6.3 Interfaces

- `Browser::frame() -> Update`: blocking. Waits for any paint and returns the whole screen. Used by
  the desktop driver, tests and start-up, so their behaviour does not change.
- `Browser::try_frame() -> Option<Update>`: non-blocking. Shows the bars and other chrome at once. While
  a paint runs and a page change is waiting, it returns `None` for that change; the page change is shown
  when the paint finishes.
- `Browser::page_ready() -> Option<Update>`: applies a finished paint or prefetch and shows the page change
  that was waiting for it. `None` when nothing is waiting.
- `Browser::set_waker(Arc<dyn Fn() + Send + Sync>)`: called by the worker after each job. The device sends
  `Ev::PageReady` into its input channel with it.
- `Browser::paint_in_flight() -> bool`: for tests and the device log.

Inside the browser module:

- `host.rs`: the worker thread, `Host::ask` (wait for a result), `Host::post` (queue, get a ticket),
  `Host::take`/`wait` (collect a ticket), `Host::build` (page build with the pump: progress and cancel).
- `page_on(f)`: run `f` on the current page on the worker and wait. Replaces `self.page.as_ref()` and friends.
- Pending changes are two: `change` (bars, sheets, keyboard, quick panel, notices) and `pending_page`
  (anything that touches the page). `touch_chrome` and `touch` add to them.
- A paint is a `Flight` (ticket, page generation, cache key). It is applied only while the generation is
  current; `page_changed()` bumps the generation.

## 6.4 Verification

- Unit tests (computer): `browser/host.rs` (job thread, order, panic handling, waker, take/wait) and
  `browser/mod.rs` `responsive_tests`: the menu is shown while a deliberately slow paint (test-only delay of
  1.5 s) is still running; a picture that the page has outrun is not shown, and the final picture equals a
  build without the delay.
- L2 desktop regression: `scripts/desktop-regression.sh` against a release `kb-desktop` built from this
  change: 82 of 82 pass.
- Build: `cargo test --workspace --exclude kb-kindle` and `cargo test -p kb-kindle` pass. The armv7 device
  build (`cargo zigbuild --release -p kb-kindle --target armv7-unknown-linux-musleabihf`) compiles; the device
  itself is not tested here.

## 6.5 Known limits

- A tap on the page content, a page turn and a link highlight call the page worker and wait for it. If a paint
  or a prefetch is running, they wait for it to finish (one paint at most, about 300 ms on the Oasis 3). The bars,
  the menu, the address bar, the keyboard, the quick panel and the settings never wait.
- Loads still run the page build on the worker while the UI thread waits; taps during the build cancel the
  remaining resource loads, as before. Network fetches keep their own thread.
- Each thread loads the font collection once (`kb-engine` keeps it per thread), so the device holds a second copy
  of the fonts (about 2 MB of font files) for the UI thread's bars. Measure RSS on the device before a release.
