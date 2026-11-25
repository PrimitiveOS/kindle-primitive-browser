# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.3.0] - 2026-11-25

### Added

- The page is painted on its own thread (`browser/host.rs`, see `docs/06-page-worker.md`). The bars, menus, address
  bar, keyboard and settings respond while a page is being painted.
- Automated device tests (`tests/device/run.py`, `docs/07-device-tests.md`): the browser can read its touch and
  buttons from test FIFOs named in its folder; absent by default.
- Under Primitive Launcher the browser follows the system layer. The library entry declares `# Wake: redraw`, and
  the browser draws its whole screen again when the launcher asks (`SIGUSR1`). `Browser.sh` ignores the signal in
  its own shell.

### Changed

- Loading a page no longer blocks the bars, the menus or the settings: the download runs on its own thread, the page
  is built on the page worker, and the chrome layers are drawn on their own worker (see `docs/06-page-worker.md`).
- A page change waits for its paint; a change to the bars, sheets or keyboard is shown at once, over the picture on
  screen. A page turn or scroll that needs a paint shows when the paint is finished.
- Under Primitive Launcher the whole browser, bars and pages, follows the system's light or dark mode, and the
  Display sheet has no Day and Night. Opened alone, Night works as before.
- Under Primitive Launcher the swipe down from the top edge opens the launcher's quick settings. The browser's own
  panel opens only when the browser is opened alone.
- The launcher's redraw after a theme switch draws the whole screen with the page waveform, without a flash. A
  second GC16 flash showed the old theme for a moment.

## [0.2.18] - 2026-11-19

- Package Browser for both ARMHF and ARMEL. The ARMEL build uses a static FBInk helper.
- Include the software and documentation licenses in both Store package variants.

## [0.2.17] - 2026-11-19

### Changed

- Licence: the code is under the PolyForm Noncommercial License 1.0.0, and the documentation and pictures under
  CC BY-NC-SA 4.0. Releases up to 0.2.16 stay under AGPL-3.0 (and up to 0.1.18 under MIT). The release package
  carries both licence files.
- The project skills moved to the shared `doannt-kindle/.agents/skills` folder.

## [0.2.16] - 2026-11-18

Device builds 0.2.8 to 0.2.15 are part of this release; they were not tagged.

### Removed
- The shell engine (`crates/kb-term`, ported from KOReader's terminal plugin) moved to Primitive Terminal, which is its own app.
- The SSH server (Dropbear and its zlib) and `deploy/set-dns.sh`. The launcher ships and runs its own Dropbear, and its DNS script does public DNS.
- Diagnostics (the menu's Diagnostics button and `kbp://diagnostics`), with its checks, Check SSH, Start and Stop SSH, Set SSH password and public DNS. They moved to the launcher's Settings > SSH & DNS, which also has the quick settings' SSH switch. The browser's quick settings have no SSH switch: Airplane, Night, Images and Settings remain.

### Added
- Quick settings like the launcher's: round buttons (Airplane, Night, Images, Settings) that fill when on, brightness and warmth rows, and a System section with CPU, memory and storage. The figures are read on each opening of the panel. The icons and figures come from `primitive-quick` in the tools submodule.
- The licence texts for the device tools (FBInk under GPL-3.0, Dropbear, zlib) are in `assets/device/licenses/`, with the source references for the FBInk and Dropbear binaries in `assets/device/README.md`. The release package and the device install carry them in `browser/bin/licenses/`.
- Start SSH shows the root password: the one last set with Set SSH password (saved in `browser/ssh/password.txt`), or the device default before one is set.
- The library icon is a portrait cover (600x920), like the book covers.
- Project skills for Kindle work in `.agents/skills/`: `kindle-device` (access, install, debugging) and `kindle-eink` (framebuffer, waveforms, page turns, FBInk).

### Fixed
- SSH could not connect after the Kindle restarted: its firewall drops incoming connections, and only a rule left by KOReader's SSH plugin had let them through. Start SSH now opens port 2222 in the firewall (Stop SSH closes it, Check SSH reports it).
- A key press on the on-screen keyboard flashed the whole row: the highlight update covered the full width of the row. Updates now carry the changed columns, and a key highlight sends only the key to the panel.
- The screen went blank when the device was held upside down: the framebuffer reports rotation 2, and the in-process path only accepted rotation 0, so updates went to FBInk, which cannot show images. Rotation 2 is drawn as it is (the panel shows the memory unturned).

### Changed
- `deploy/install-ssh.sh` stops the browser before replacing its files; replacing a running program's file on the FAT storage can crash it (SIGBUS).
- The browser runs under Primitive Launcher: with `PRIMITIVE_LAUNCHER=1` set by the launcher's supervisor, `deploy/Browser.sh` does not stop, restore or open the stock UI, and the browser returns to the launcher at once on exit.
- The on-screen keyboard uses the shared key style from `primitive-keyboard` in the tools submodule, as the launcher does.
- `scripts/build-device.sh` installs over SSH when the Kindle is on Wi-Fi or `KINDLE_HOST` is set; over USB only when the Kindle is mounted.
- The release and CI workflows check out the `tools/` submodule, which the build depends on.

## [0.2.7] - 2026-11-17

### Fixed
- Start SSH failed while another SSH server (KOReader's Dropbear) held port 2222, and Stop SSH did nothing, because it only acted on the browser's own pid file. Start SSH now refuses when the port is in use. Stop SSH stops every dropbear process. The SSH status reads the listening socket.

### Changed
- The browser's own Dropbear (`/mnt/us/browser/bin`, with the browser's host key) is the SSH server.

## [0.2.6] - 2026-11-17

Device builds 0.2.2 to 0.2.5 are part of this release; they were not tagged.

### Added
- Display sheet, SOLID TEXT (on by default, saved as `solid_text`): near-black page pixels are drawn pure black and near-white pure white (the two panel steps next to each: 0, 17, 34 -> 0 and 221, 238, 255 -> 255). Anti-aliased edges keep their grays.
- Display sheet, BOOK LINES (on by default, saved as `book_lines`): paragraphs are spaced by one blank line, so the text of the pages sits on one grid.
- Page turns like a book reader's: no overlap. A turn scrolls to the next text line on the page's line grid, so the lines of the new screen land on the rows of the old one. Pages without text lines turn by a whole screen.
- Display sheet, PAGE TURN DITHERING (saved as `page_dither`): On dithers page content in the browser with an ordered 8x8 pattern. Off by default.
- The browser draws page turns in-process: it writes the framebuffer and sends the e-ink update itself. A turn reaches the panel about 45 ms after the button press, down from 280-1340 ms.
- The device tools and fonts ship with the browser: FBInk (`/mnt/us/browser/bin/fbink`), Dropbear and its zlib, and Noto (`/mnt/us/browser/fonts`). Nothing else needs to be installed on the Kindle. See `assets/device/README.md`.
- `deploy/install-ssh.sh` copies the fonts and device tools by SHA-256, so an unchanged install copies nothing. The release package includes them too.
- `tuning.txt` in the browser folder can change the prefetch delay and in-process drawing without a rebuild (for measurement).

### Fixed
- Every page turn flashed (black in Day, white in Night). Sites set text in a dark gray such as `#111`, so each letter was a gray level; the panel reaches a gray by passing through the opposite extreme, so every letter flashed. Measured on the Oasis 3: pages with pure black or white letters did not flash in AUTO, GL16, REAGL, GC16 partial or DU. SOLID TEXT draws such letters pure. Button presses were checked and were not the cause.
- Page content is cut to the panel's 16 gray steps before it is sent, which cut the driver's transition count from about 2,800-3,350 to about 100-580 (stock reader: 139-484), so the driver's own flash pass no longer runs. `KB_QUANTIZE=0` turns it off.
- The browser could stop updating the screen with page dithering on: the FBInk build on the Kindle cannot dither images, so each turn failed. Dithering now runs in the browser.
- The screen's orientation was read from FBInk, which crashed on `-e` with the FBInk 1.25 build (the browser logged `framebuffer None`). The bundled FBInk build reads it correctly.
- Exit needed two taps: the first Exit crashed while the browser was being dropped (SIGBUS), so the launcher restarted it. The browser now leaves right after saving and clearing the screen.
- Opening the address bar no longer drops the cached screens; the screen before the current one is prefetched as well as the next.
- Prefetch waits for the panel to finish a page turn before it renders the neighbouring screens.

### Changed
- The SSH server is the browser's own Dropbear, run from `/mnt/us/browser/bin` with its own host key in `/mnt/us/browser/ssh/`. The first Start SSH copies the host key that the KOReader install used, so the server keeps its fingerprint. Its pid file is `/tmp/dropbear_browser.pid`.
- The Set SSH password help no longer shows the default password.

## [0.2.1] - 2026-11-17

### Added
- Display sheet, screen refresh section: page refresh (AUTO by default, the stock EPUB reader's mode; or GC16, GL16, REAGL, DU), bars and taps refresh (DU by default, or GL16), and a flash on each new page (off by default). Saved in `settings.tsv` as `page_wave`, `ui_wave` and `flash_on_load`.
- Rendered-screen cache: the last three screens of a page are kept, so turning back to one is quick. The next screen is rendered while the device is idle (prefetch), so the next page turn is faster.
- Diagnostics: Check SSH reports whether the SSH server is listening on port 2222, by reading the kernel's socket table, without starting or stopping anything.
- Deploy over SSH: `deploy/install-ssh.sh` copies the browser, the library launcher and the icon to the Kindle over Wi-Fi, through one shared connection (one password prompt), and checks the new binary's SHA-256 on the device.
- Crash log: the device build records the signal and the faulting address before it exits.
- Forms: checkboxes and radio buttons toggle on tap and are sent only when checked; disabled fields are not sent; readonly fields are sent; password fields show stars; Done commits a field without sending its form (the form's submit button sends it).
- `XMLHttpRequest` as a wrapper over `fetch` (readystatechange, load, error, loadend).
- Full `matchMedia` queries: lists, `and`, `not`, `only`, size, orientation, colour scheme and pointer.
- `localStorage` is kept per site in the data folder and survives a restart (1 MB per site).
- Images are off by default for web pages; each image shows its alt text in a box.
- Find in page: a Find button in the menu opens the keyboard; each Find scrolls to the next match.
- Scripts on or off per site (Display sheet). Off removes the site's scripts and shows its `<noscript>` text.
- Select lists: a tap opens the options in a sheet, and the chosen one is sent with the form.
- Page scripts get `input` and `change` when a field, checkbox, radio or select is committed, and a cancelable `submit` before a form is sent.
- `history.pushState` and `replaceState` change the address shown, and the page's history entry follows (no load).
- Session, bookmark, history, settings, scripts-off and saved-page files are written atomically (temporary file, flush, rename). A saved page that cannot be written is reported to the reader.
- `setInterval` fires a bounded number of times while a page settles (three), and `clearInterval` stops it.
- Offline copies keep the page's stylesheets and images (inside the saved HTML) and drop its scripts; the copy opens without a network.
- `pushState` adds a history entry and `replaceState` replaces it. Back and Forward to an entry the page pushed fire `popstate` with the pushed state, without loading the page again.
- Desktop memory and page-turn measurements in `docs/PERFORMANCE.md`.
- Device values (screen size, touch name, page-button keys, battery and charger paths) are in one per-model table in `kb-kindle`; the Oasis 3 values are unchanged.
- A ten-site matrix (`--network`), with dated results in `docs/SITE-MATRIX.md`.
- A basic `MutationObserver`: attribute and child-list changes made through the DOM methods are delivered to the observer.
- Diagnostics page (`kbp://diagnostics`, Diagnostics button in the menu). Run checks runs eight read-only checks through the shell engine (shell, system, storage, network interfaces, DNS, hosts file, browser files, processes) and shows each result. Results are also logged as `[diag]` lines. Set public DNS points the device at 1.1.1.1 and 8.8.8.8 and patches the Wi-Fi DHCP script so the setting survives reconnects and restarts (originals saved to `/mnt/us` first), and Restore DNS puts them back. Both run fixed commands through the shell engine and log `[fix]` lines. Start SSH runs an SSH server on port 2222 with a new random root password, shown on the page. Stop SSH stops the server and its sessions. Set SSH password types the root password on the keyboard; it is checked, set with chpasswd, and never written to a URL or the log.
- `kb-term` crate: a shell command engine. It runs a shell on a pseudo-terminal, returns each command's output, exit status, and whether it timed out, and stops a command that runs too long with Ctrl-C. It is the base for the troubleshooting tools. Ported from KOReader's terminal plugin (AGPL-3.0); see the header of `crates/kb-term/src/lib.rs`.

### Changed
- Licence changed from MIT to GNU AGPL-3.0 (`LICENSE`, `Cargo.toml`, README). Releases up to 0.1.18 stay under MIT.
- A finished page no longer gets a full GC16 flash, and the periodic full flash and the two-second redraw are removed. Page turns use the stock reader's partial refresh (AUTO), so the screen changes by pixels rather than flashing.
- Each request looks up the site's address once per minute at most. The loopback check and the HTTP client
  share that lookup, and a page's images reuse it, instead of resolving the same host again for every request.
  A failed request drops its remembered address.

### Fixed
- Dark pages no longer snap to a black screen: night-mode values below 16 are treated as black.
- A page that finished loading flashed the whole screen; the flash on load is now off by default (see Display).
- After a Wi-Fi drop, pages whose site publishes IPv6 addresses kept failing with "Address family not
  supported by protocol (os error 97)", even after reconnecting, so Reload could not recover. The HTTP
  client now uses IPv6 only where the system can open IPv6 sockets. The Kindle cannot, so there it connects
  over IPv4 only.

## [0.1.18] - 2026-11-16

### Changed
- Licence: MIT (`LICENSE`), set in `Cargo.toml` and the README.

### Removed
- The bookmark that shipped with the app is gone. A new install starts with no bookmarks.

### Added
- Project documentation for publishing: a full README (status, features, install, build, testing,
  structure), `docs/INSTALL.md` for installing the release package without building, `docs/VERSIONING.md`
  for version numbers, tags and the release steps, `docs/DEVICES.md` for the support matrix and the plan
  for other Kindle models, and `CONTRIBUTING.md`.
- Release packaging: `scripts/package-release.sh` writes `dist/primitive-browser-<version>.zip` with its
  SHA-256 file. The package holds `browser/` and `documents/`, ready to copy onto the Kindle.
- GitHub Actions: `ci.yml` runs unit tests, launcher checks, the desktop regression with screenshots and
  the ARMv7 build on every push and pull request; `release.yml` publishes the package when a `v*` tag is
  pushed, after checking the tag matches `Cargo.toml`.
- `deploy/install.sh` finds the Kindle on macOS and Linux, or at the path given in `KINDLE`.

### Fixed
- Pages showed their dark theme on a light screen. A stylesheet link with a `media` query for the dark colour
  scheme or for print was applied anyway. Such links are now skipped for web pages.
- Links from search engines did not open. A page that redirects with its script (`window.parent.location.replace(...)`,
  as DuckDuckGo's result links do) or with `location.href` now loads the target in place of the redirect page.
  Meta-refresh redirects (up to three seconds) load too. `window.parent` and `window.top` are the page's window.
  Up to five redirects are followed, so a loop stops. Covered by `script_navigation_follows_redirects`.
- The quick settings panel showed "not charging" while the Kindle was charging. The driver read the
  framework's `isCharging` property first, and its answer 0 ended the search before the charger's own
  flag was read. It now checks the property, charger flag and charger status: either integer value 1
  or status `Charging` reports charging. The log records the raw values and charger online state,
  plus a sorted inventory of the kernel power supplies with their type, status, online and present
  values for diagnosis.

### Added
- Touch and the page-turn buttons follow the orientation the framework shows (its accelerometer
  property, `com.lab126.winmgr`), read once a second. FBInk's framebuffer rotation is logged beside
  it, so a disagreement with the text on screen shows in the log (`display turned: rota N
  (framework …, framebuffer …)`). A tilt while lying on a bed does not flip the controls.
- The browser runs over the Kindle's UI without restarting the framework (its restart took about 9 s
  in the device log): the status bar is hidden while it runs, and on exit it is shown again and the
  library is opened. The window manager is left running, since pausing it stopped the framework from
  turning the screen. The library open is retried every quarter second instead of
  waiting a fixed time. If the library shows a blank screen, create the file `stop-framework` in the
  browser folder on the Kindle: the launcher then goes back to stopping the framework.
- Left-handed (upside-down) portrait: touch and the page-turn buttons follow the turn, in our own
  driver (KOReader does not do this for this model). The touch controller reports in the panel's
  upright frame, so each touch is turned when the Kindle is upside down, and the top and bottom
  page-turn buttons trade their directions, so "next" stays on the reader's side. The launcher log
  shows each touch's native and screen coordinates and the turn.
- Quick settings panel, drawn from the top like the Kindle's own: a swipe down that starts within
  40 px of the top edge opens it. It shows the clock and date, battery, Wi-Fi (an On/Off switch),
  frontlight brightness and warmth as 25-step bars with − and + at the ends, and a Close handle.
  A tap on a bar or a button sets it and the panel stays open; a tap below the panel or a swipe
  closes it. The device values come from the Kindle's system properties (`lipc-get-prop` /
  `lipc-set-prop`, read in `crates/kb-kindle/src/device.rs`); the desktop driver shows defaults
  and its `edge` command opens the panel. The lipc names follow KOReader and are not yet
  confirmed on the device. The top row holds the clock on the left, and on the right the Wi-Fi
  button (slashed when off) and the battery: a battery drawn to the charge level in black, with the
  charge number split at the level's edge (white over the black part, black over the white part),
  and a bolt beside the number while charging. The
  charging state combines the Kindle's `isCharging` property, charger sysfs flag and status text;
  the launcher log records the battery, charging result and raw inputs each time the panel opens.
- Sessions: the open tabs, each tab's history and scroll positions, and the active tab are saved
  whenever a page loads or a tab closes, and when the browser leaves. The next time it opens
  (from the Kindle library or after a crash), the same tabs come back.
- Exit in the menu's title strip closes the browser and returns to the Kindle library. The open
  tabs are kept for the next time. The Kindle has no background app switching here (the framework
  owns the screen and touch input), so the browser does not stay running behind the library.
- Keyboard third layer "#+=": the symbols layer has a "#+=" key; the new layer adds brackets,
  `# % ^ * + =`, `_ \ | ~ < > € £ ¥ •`, and more punctuation, with "123" and "ABC" to switch back.
- The menu title shows the build version ("Menu  v0.1.1"). Each device build is a new patch
  version (for example 0.1.0 -> 0.1.1); `scripts/build-device.sh` bumps it, builds, and installs,
  and undoes the bump if the install fails.
- Long-list tests: 40 seeded bookmarks (the `SEED` option of the regression runner) cover the
  address suggestions (scroll, and a tap far down the list), filtering by typing, the home grid
  scrolled to its last tile, and the bookmark manager (scroll, and remove a row far down).
  `gallery_long_lists` saves the scrolled states as screenshots.
- Address bar selection: opening the address on a page selects the URL, so the first key
  replaces it instead of appending. While it is selected, the suggestions list the bookmarks and
  recent pages. Tapping a character places the caret and clears the selection.
- Test pipeline `scripts/test-all.sh`: build, unit tests, desktop regression, launcher tests and
  the Kindle build, with the evidence kept in `artifacts/` and a summary in `artifacts/REPORT.md`.
- Screenshots in the repository: each regression scenario has a folder (`script.txt`, `log.txt`,
  `final.png`); the `gallery_pages` scenario captures every page and sheet as named shots;
  `artifacts/desktop/INDEX.md` lists them. The driver gained a `shot <name>` command.
- Menu redesigned as a four-column grid of tiles (PAGE, SAVE, BROWSER), with a state caption on
  each tile. Night mode is removed from the menu (it is in Display), and the menu no longer has a
  Done button; tap the page above the menu to close it. Exit browser stays in the title strip.
- Dependable reading/navigation milestone with explicit acceptance criteria in
  `docs/04-roadmap.md`, plus a Kindle interaction and e-ink guide in `docs/05-ui-ux.md`.
- Article section links: encoded Unicode IDs, legacy named anchors, empty/top fragments,
  missing-target behavior, and Back/Forward positions without rebuilding the live document.
- Pinned sheet headers: Done stays visible in Menu, Display and Tabs; Exit browser remains
  visible in Menu after scrolling. Tab close targets are 68 px with title space reserved.
- Desktop `assert` commands for URL, title, scroll range, tab count, Back/Forward state and
  visible controls; failures terminate the scenario.
- Documentation set in `docs/`: requirements, system architecture, test and validation
  pipeline, and this changelog.
- Desktop regression suite `scripts/desktop-regression.sh` (67 scenarios on local fixtures,
  including 13 navigation/state/sheet cases; opt-in live-site checks with `--network`).
- Display sheet (menu > Display): text size steps (80% to 200%) with A- and A+, page width
  (Narrow, Medium, Wide) and Day or Night. Changes apply at once and the sheet stays open.
- Page width setting (`page_width` in `settings.tsv`). Web pages lay out at 600 CSS px
  (Medium) by default, with 412 px (Narrow) and 800 px (Wide) available; device usability
  remains to be measured.
- Bookmark manager (`kbp://bookmarks`): each bookmark shows its title and address, with open,
  move up, move down and remove buttons.
- Home sections fold away: tap the BOOKMARKS or RECENT heading to show or hide the list. The
  choice is kept in `settings.tsv`.
- Images switch in the menu (PAGE > Images, On by default). Off answers image requests
  without the network or the decoder, so text-only reading uses less memory; the page is
  rebuilt when it changes. Videos are not played: the engine has no video support, with
  images on or off.
- Launcher test `scripts/launcher-stub-test.sh` (stub commands; ten checks).
- Unit tests for URL handling, the blocklist, image detection, title and script rules, HTML
  preparation and PNG encoding.
- Desktop driver commands: `tapsel`, `longsel`, `tapkey`, `text`, `where`, `linkat`,
  `visible`, `count`, `tabstep`, `longpress`.
- Device log now records resident memory (`rss`) after each action.

### Changed
- The menu's title strip has a border on top, between the page and the menu, instead of a line
  under the title.
- A finished page gets one full refresh of the screen (GC16 flash), and so does the last image
  arriving. There is no manual refresh control in the browser.
- The address field and the keyboard redraw with the text waveform (GL16) instead of DU. DU left
  old text and the caret behind on e-ink.
- Renamed to **Primitive Browser**: library launcher `documents/Primitive Browser.sh` (the
  installer removes the old `KB Browser.sh`), user agent `PrimitiveBrowser/0.1`, and the logo
  (`assets/primitive-icon.png`, from `primitive-ai-website/public/prai-icon.png`) on the home
  screen. The logo is built into the binary, so it shows offline.
- `data:` URLs now load (base64 and percent-encoded). Before, they were answered empty, so inline
  images on web pages were blank.
- Address bar while typing: the URL field is in the same 96 px bar as the page's address bar,
  with Cancel where the reload button sits. The Go button in the header is removed; the keyboard's
  Go key submits. The suggestions are a separate list between the bar and the keys, and they
  scroll with a swipe (the bar and the keys stay in place). The list keeps up to 60 matches,
  bookmarks first (the list was limited to 6).
- The driver's `visible` check now means the element's centre is on the part of the screen the
  user sees: below the address bar and above the toolbar, the open sheet's body, or the keyboard's
  list above the keys. It used to count any element with a box, including elements scrolled away
  or under the keys.
- Home screen redesigned: bookmarks as a four-column grid of tiles (the menu's tile style), recent
  pages as rows with a title and a URL, and each section has a text control, Show or Hide, in
  place of the chevron. Empty states are padded. "Manage" opens the bookmark page. The two
  bottom buttons are removed (the menu has Bookmarks and Saved pages).
- Display and Tabs sheets no longer have a Done button. Tap the page above a sheet to close it.
- `kb-app/src/lib.rs` (about 2,100 lines) is split into modules by responsibility: `browser/`
  (state, navigation, loading, tabs, input, actions, library, keyboard, frame, internal pages, driver),
  `settings.rs`, `storage.rs`, `html.rs`, `addr.rs` and `layout.rs`. The behaviour is unchanged:
  all 67 regression scenarios and the unit tests pass. The layout is described in
  `docs/02-architecture.md` §2.12.
- Web pages use the selected 412/600/800 CSS-px width and scale to the panel; default is
  Medium (600). JavaScript viewport dimensions, pixel ratio and basic width queries agree
  with the renderer instead of always reporting 412 px. Chrome and internal pages keep
  device pixels.
- Address bar is 96 px (was 120 px); bottom toolbar is 104 px (was 130 px).
- Interface redesigned to a single layout system (`crates/kb-app/src/ui.rs`): grey pill
  address field with a site badge, icon-only bottom toolbar with a tab count and a
  three-dot menu, grouped menu with icons and dividers, card-grid tab switcher, and
  folding bookmark/recent sections on Home.
- Touch: a tap is a touch under 800 ms with less than 40 px of movement; a long press is a
  touch held 800 ms or more; a vertical swipe scrolls from 80 px; horizontal swipes need
  180 px. A tap during a document download no longer cancels it.
- Image repaints during loading are limited to once every 1.5 s.
- Cache budget raised to 200 MB; JavaScript heap raised to 48 MB; sub-resource limit 4 MB;
  document limit 6 MB.
- Default bookmark is added once for existing devices (`defaults_v2` marker), so removing
  it does not bring it back.
- Menu and tab sheets scroll.

### Fixed
- JavaScript-heavy pages (React and Next.js, jQuery) failed or went blank. Found on tinhte.vn
  from the device log and fixed at the cause, each with a regression check (`dom_api_for_pages`):
  - `innerHTML` assignments did nothing: the document had no HTML parser for fragments. The
    Blitz HTML parser is now installed.
  - `getAttribute` returned `undefined` for a missing attribute, so every `appendChild` was taken
    for a fragment and dropped (introduced earlier in this release). It returns `null`.
  - Text nodes were hidden from scripts (`childNodes`, `firstChild`, `nextSibling`), so React's
    hydration never matched the server markup (React error 418) and the page was cleared.
  - Missing DOM: `document.nodeType`, `URL`, `referrer`, `currentScript`, fragments, `cloneNode`
    (native, deep), `attributes`, `ownerDocument`, `isEqualNode`, `dispatchEvent`, siblings,
    `HTML*Element`/`Text`/`SVGElement` classes for `instanceof`, `performance.clearMarks`, layout
    stubs. jQuery 1.11 failed on the first of these (`createElement` of undefined).
  - `fetch` ignored its method, headers and body, did not resolve relative addresses, and had no
    response content type. It now sends them through a new `kb_net::request`.
  - A script answered with an HTML page (an error page) was run as JavaScript; it is skipped.
  - Removed nodes reached by later script calls made Blitz panic, which replaced the whole page
    with an error page. The DOM calls and each page-building step are contained, and Blitz is
    patched to ignore removed nodes (`vendor/blitz-dom-0.3.0-beta.4`, see `vendor/README.md`).
  - Errors from event handlers, timers and `console.error` now log their first stack frames.
- Slow page loads (about 8 s on the device for a script-heavy page): external scripts were
  fetched one at a time. They are now fetched six at a time and still run in document order
  (regression `scripts_run_in_document_order`). On the Mac, tinhte.vn took 14 to 24 s before and 4 to 6 s after (network-dependent).
- Saved pages did not load their own scripts and stylesheets (the device log showed
  `kbp://saved/...: unsupported scheme`). The saved-page metadata is `url title`, but the file
  writer turned the tab between them into a space, so the lookup found no URL and the page fell
  back to its internal address. The metadata now splits on the first space, which reads the
  entries already saved on the device (regression `saved_page_runs_its_scripts`).
- A page restored with Back took its internal address as its base for relative links, so its
  links stopped resolving. Internal addresses no longer change the base; only web addresses do
  (unit test `internal_location_keeps_the_web_base_for_relative_links`, regression
  `saved_page_links_after_restore`).
- Pages that check for IndexedDB failed with `ReferenceError: IDBIndex is not defined`. The
  IndexedDB names now exist, and opening a database fails quietly (regression
  `indexeddb_feature_checks_do_not_throw`).
- Address suggestions showed the bookmark icon for recent pages too. Recent pages now have their
  own icon.
- JavaScript error reports now include the first three stack frames, so the device log shows
  where an error was thrown (script URL and line).
- Tapping a suggestion while typing an address did nothing: suggestion rows are browser actions,
  but the keyboard sent them to the key handler, which ignored them. They now open the page
  (regression `suggestion_tap_navigates`).
- Menu rows for Bookmarks, Saved pages and other `kbp://` pages did nothing: the action table
  ignored them. Sheet links to internal and web pages now open them (regression
  `menu_items_navigate` checks every tile).
- Page scripts saw a stale `location.href` after a fragment jump: `href` is a getter, which
  `Object.assign` does not copy. It is now set explicitly (regression `fragment_updates_location_href`).
- Switching tabs cleared the page but not its HTML, so a cancelled load left the previous tab's
  source under the new address; "Save for offline" or a text-size change could then store or
  rebuild the wrong page. The source and reader state are now cleared on switch.
- The reader view laid out at device width, so the page-width setting did nothing while reading.
  It now uses the selected web width.
- Reflow, text-size and reader rebuilds recorded the page in history again. Visit recording
  now happens once per navigation (`record_web_visit`).
- The launcher stub test could block forever if the launcher hung; it now fails after 30 s.
- Back/Forward added saved offsets to already-scrolled live pages; restoration now sets an
  absolute position and swaps the document with its matching source and reader state.
- Opening a new tab stashed the original tab twice, discarding its history. Tabs now retain
  history and reading positions; switching clears live content from the other tab. Closing
  the final tab resets to one Home tab and closing cards keeps the switcher usable.
- Display changes could rebuild the wrong source after instant Back, lose reading view, or
  restore a cached layout using old settings. Source/view state travels together and reflow
  invalidates the previous layout.
- Unit tests no longer fail to compile when Settings gains fields: fixtures use full defaults.
- Keyboard caret regression now submits to a local form fixture instead of a live search
  service. The saved-page regression opens the saved document and checks its title.
- Regression output directories are created when `KB_TEST_OUT` is supplied. Launcher checks
  now wait for the stale-marker recovery run and verify its marker is cleared (ten checks),
  avoiding cleanup while that run is still writing logs.
- Following a link with the bars hidden showed no address bar, and the progress strip landed
  on page content. The address bar now shows the new URL before the load starts, and the
  progress strip sits under it.
- Exit browser could leave the Kindle on the app drawer, which showed a blank screen. The
  launcher now waits for the framework to start and then opens the library home.
- The home screen had two search boxes (the address bar and a search pill). The pill is removed.
- Taps on a scrolled page hit the element at the same height at the top of the page instead
  of the one under the finger (hit testing now adds the scroll offset).
- The top of a page was hidden under the address bar; pages are now painted below it while
  the bars are shown, and touches are mapped back to page coordinates.
- Mobile-only page sections (for example a continue-reading control) were hidden because the
  layout width was desktop-sized. Fixed by the phone-width layout.
- Streamed server-rendered content (hidden `S:n` chunks) is moved into place when a page's
  own swap script fails.
- A tap on a slightly moving finger was read as a swipe; a slow tap was read as a long press
  and opened a new tab.
- Module scripts (`type="module"`) were being run as classic scripts, which produced errors.
- New tab kept the previous tab's address.
- Kindle UI could be left stopped after the browser was killed; the launcher now has a
  watchdog that restores it, and restores a stale stop on the next launch.
- Missing DOM methods (`getElementsByClassName`, `classList`, `dataset`, `matches`,
  `closest`, `contains`, `insertAdjacentHTML`, sibling properties) and browser APIs
  (`AbortController`, `crypto`, `URL`, `TextEncoder`/`TextDecoder`, `btoa`/`atob`,
  `performance`, `matchMedia`, `ReadableStream`, `IntersectionObserver`, `DOMContentLoaded`
  and `load` events).
- Menu on the device flashed and disappeared. Every menu choice closed the sheet, and each
  sheet redraw used the fast DU waveform, which leaves ghosting and erased text on large
  grey areas. Now toggles (night mode, text size, bookmark, save for offline, clear cache,
  clear history) keep the menu open and show the new state; only navigation closes it.
  Sheet, bar and notice redraws use GL16 (text quality) instead of DU.
- The menu had no visible exit: the Exit row was below the fold of the 780 px sheet. Exit is
  now a button in the menu's title strip, always on screen.
- Device log: every sheet tap and every chrome action is logged with the sheet state, for
  example `[ui] sheet=Menu tap (x,y) -> Some("kb:night")` and `[ui] action kb:night (sheet=…)`.

### Known issues
- Whether the screen sometimes stays stale after a page loads is not confirmed. The full refresh
  on load is the control in place; it needs checking on the device.
- A site's own poll widget (tinhte.vn) still reports `Could not find job from response`: its
  batching client does not find its job in the batch reply. The article renders; only the poll
  is affected. Not investigated further.
- Layout queries from scripts (`getBoundingClientRect`, `offsetWidth`, ...) report zero: the page
  layout is not exposed to scripts. Code that only measures and continues works.
- The desktop driver's `tapsel` cannot target an inline link (a link inside a sentence): the
  element has no box of its own. Taps on the device still work for inline links.
- Stock Kindle keyboard access remains unknown; the browser uses its own keyboard.
- Latency, memory and e-ink behaviour are not yet measured on the device.
- Touch gestures and the progress strip have not been confirmed on the touch panel.
- JavaScript coverage is partial: no ES modules, XMLHttpRequest, WebSockets, workers, canvas,
  WebGL or video (see requirements §1.4).
- Form controls other than text fields (checkbox, radio, select, file) are not supported.
- Done/Go commits a page field and submits immediately; multi-field editing, form events,
  and POST reload/history behavior need improvement. POST coverage uses text fields.
- Saved copies retain HTML only. Offline assets, atomic data writes and stronger session
  durability remain future work; basic session checkpoint/resume is implemented.
- Fragment navigation updates location reads but does not dispatch hashchange/popstate.
- Blink-level CSS fidelity is limited by Blitz 0.3.0-beta.4.

## [0.1.0] - 2026-11-16

First complete build of the browser for the Kindle Oasis 3, and the desktop build used to
test it. Work done in milestones and rounds, summarised:

### Milestone M0: feasibility (2026-11-15)
- Rendering spike: Blitz renders HTML and CSS to 8-bit gray; the ARMv7 static build works.
- Display on the device through the device's FBInk (`libkh`); a grayscale PNG shows on the
  panel.
- Network on the device: TLS with bundled roots; fetch, render and display in one run.

### Milestone M1: desktop browser (2026-11-15)
- Workspace split into `kb-net`, `kb-engine`, `kb-app`, `kb-desktop`, `kb-kindle`.
- Links, scroll, history, toolbar, on-screen keyboard for the address.
- Parallel sub-resource loading, so a Wikipedia article loads in about a second on the
  desktop.

### Milestone M2: device lifecycle (2026-11-15)
- Launcher stops the Kindle UI while the browser runs and restores it on exit.
- Crash watchdog restarts the browser up to three times.
- Refresh policy: text-quality waveform for pages, periodic full flash to clear ghosting.

### Milestone M3: daily use (2026-11-15)
- Persistent cookies; disk cache with offline fallback; bookmarks; reading view; saved
  pages for offline; tracker blocklist.

### Milestone M4: JavaScript (2026-11-15)
- QuickJS (rquickjs 0.14) with the DOM bridge and browser-side prelude; scripts run at load;
  click listeners with `preventDefault`; timers; synchronous `fetch` under a Promise.
- Vendored `rquickjs-sys` with musl bindings so the device build links.

### Milestone M5: hardening (2026-11-15)
- JavaScript heap cap and interrupt after 3 s; sub-resource and document size caps; panic
  guard around each event; plain-language error pages.

### Interface rounds (2026-11-15 to 2026-11-16)
- Cached interface layers so a tap re-renders only what changed; partial refresh with the
  fast waveform for bars and keyboard.
- Tap highlight before the action runs; progress strip while loading; deferred images;
  notices above the toolbar.
- On-screen keyboard after the stock layout structure (layout only, not code); caret;
  suggestions; input mode for page fields with Done; GET and POST form submission.
- Tabs, menu and reading settings (text size, night mode); long press to open in a new tab.

### Device-facing fixes (2026-11-16)
- Launcher watchdog and stale-stop recovery after the browser was killed.
- Phone-width layout, streamed-chunk fallback, and the scroll hit-test fix (see Fixed above).
