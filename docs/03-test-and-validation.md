# 3. Test and validation pipeline

Latest release: 0.2.1 (2026-11-17). Requirements are in
[01-requirements.md](01-requirements.md); the design is in
[02-architecture.md](02-architecture.md). Each test level below states what it proves, how to
run it, and what result counts as a pass.

## 3.1 Levels at a glance

| Level | What it checks | Where it runs | How to run | Pass rule |
|---|---|---|---|---|
| L1 Unit | Logic plus fixed fragment layout: URLs, blocklist, script rules, HTML preparation, PNG encoding, anchors | Computer | `cargo test --workspace --exclude kb-kindle` | All tests pass |
| L2 Desktop regression | Browser behaviour end to end: navigation, forms, DOM and JavaScript APIs, tabs, bookmarks, reading view, menus, keyboard, scrolling, hit testing; a gallery of every page and sheet as screenshots | Computer, against local fixtures | `scripts/desktop-regression.sh` | 82 of 82 scenarios pass; screenshots in `artifacts/desktop/` |
| Test pipeline | All of the above in one run, with the evidence kept in the repository | Computer (and the Kindle build, when cargo-zigbuild is installed) | `scripts/test-all.sh` | `artifacts/REPORT.md` shows every step as PASS |
| L3 Live-site smoke | Real sites still load (Hacker News, Wikipedia) | Computer, needs internet | `scripts/desktop-regression.sh --network` | Both pass; a failure here is reported but may be the site, not the browser |
| L4 Launcher | Normal run, killed launcher, and stale-marker recovery | Computer, with stub commands | `scripts/launcher-stub-test.sh` | 10 checks pass |
| L5 Device validation | The product on the Kindle: touch, display, performance, memory, durability | Kindle Oasis 3 | Checklist in §3.5 | All must-pass items pass; evidence attached |
| L6 Build and install | The device binary builds, is ARMv7, and matches the launcher | Computer | `cargo build --release --workspace`, `cargo zigbuild ... armv7-unknown-linux-musleabihf`, `sh deploy/install.sh` | Build succeeds; `file` reports ARM; install reports the expected size and start page |

Run levels L1 and L2 for browser-state/engine changes, and L4 for launcher/lifecycle changes
and release candidates. L3 runs before a release. L5 runs before a release
and after any change to input, display or the launcher.

## 3.2 Computer tests

### 3.2.1 Driver commands

`kb-desktop` reads one command per line from stdin and writes the screen to `$KB_OUT` after
each action. Every action prints one line to stderr:

```
[label] action <ms>, frame <ms> | <title> | <url> | scroll <y> / <content height> | ...
```

Commands used by the suite:

| Command | Effect |
|---|---|
| `open <url or text>` | navigate (text becomes a search) |
| `tap <x> <y>`, `scroll <dy>`, `page <+1/-1>`, `longpress <x> <y>` | raw input in device pixels |
| `tapsel <css selector>`, `longsel <css selector>` | scroll the element into view, then tap or long press its centre |
| `tapkey <char>` | tap a keyboard key (`space` for the space bar) |
| `action <name>` | run a named browser action (`mark`, `menu`, `reader`, `night`, `display`, `font:+`, `save`, `saved`, `marks`, `tabs`, `newtab`, `closemenu` ...) |
| `back`, `fwd`, `reload`, `tabstep <±1>` | navigation |
| `text <selector>` | print an element's text (`[text] <sel>: ...`) |
| `count <selector>` | number of matching elements |
| `visible <selector>` | 1 when the first match's centre is on the part of the screen the user sees (below the address bar and above the toolbar, the open sheet's body, or the keyboard's list above the keys); 0 when scrolled away or hidden |
| `where <selector>`, `linkat <x> <y>` | debugging: screen centre of an element; link under a point |
| `assert url/title <value>`, `assert tabs <count>`, `assert back/forward <true/false>` | fail the process if navigation state differs |
| `assert scroll <min> <max>`, `assert visible <selector>` | check absolute position or the selected control's screen center |
| `shot <name>` | wait for pending images and notices, then save the full screen as `$KB_SHOTS/<name>.png` |
| `restart` | save via Exit, create a new browser instance, and resume the persisted session |
| `quit` | end |

Environment: `KB_FONTS` (comma-separated font files), `KB_DATA` (data folder, use a fresh
temporary folder per test), `KB_OUT` (frame PNG), `KB_SHOTS` (folder for `shot` PNGs),
`KB_PROGRESS=1` (print load progress), `KB_KEEP_FIRST=1` (keep the frame drawn right after an action).

### 3.2.2 Fixtures

`tests/fixtures/` holds original test pages and `server.py`, a local server that:

- serves the pages (unknown paths return 404);
- returns a small page for `/search`, so GET form submissions have a target;
- echoes the `city` field on `POST /submit`.

`navigation.html` supplies a long document, mutable DOM state, encoded Unicode/plus/colon
fragment IDs, a legacy named anchor, a responsive-width marker, and a JS viewport probe.

The fixture text is original and carries no third-party content.

### 3.2.3 The regression suite

`scripts/desktop-regression.sh` starts the fixture server on port 8780 (or `KB_PORT`), runs
each scenario in a fresh data folder, and checks the log for expected substrings. It stops
the server when it finishes. Scenarios:

| Group | Scenarios |
|---|---|
| Pages and navigation | `home`, `js_button_keeps_page`, `link_navigates`, `back_and_forward`, `reload`, `not_found_shows_error`, `long_press_opens_new_tab` |
| Reading position and page ownership | `history_restores_absolute_scroll`, `tabs_preserve_history_and_scroll`, `close_active_and_last_tab`, `cached_source_matches_document`, `reader_survives_back_and_reflow`, `cached_layout_uses_current_settings` |
| Section links | `fragment_preserves_dom_and_history`, `fragment_updates_location_href`, `initial_fragment_and_named_anchor`, `missing_fragment_keeps_position`, `query_change_loads_new_document` |
| Persistent sheet controls | `menu_controls_stay_visible`, `tab_controls_stay_visible` |
| Forms and input | `form_get_submit`, `form_post_submit`, `keyboard_caret_insert` |
| DOM and JavaScript | `dom_apis`, `js_platform_apis` |
| Reading, saving, bookmarks, tabs | `reading_view`, `save_offline_and_reopen`, `bookmark_add_and_list`, `new_tab_and_switch`, `session_tabs_survive_restart`, `menu_exit_control_visible`, `tabs_sheet_opens`, `home_single_search_box`, `home_section_toggle`, `bookmark_reorder_and_remove` |
| Menu and settings | `menu_scrolls`, `text_size_and_night`, `address_bar_hint_on_home`, `menu_toggle_keeps_sheet`, `images_on_by_default`, `images_off_skips_fetch`, `menu_images_row_keeps_sheet`, `menu_navigation_closes_sheet`, `menu_exit_button` |
| Live sites (with `--network`) | `hacker_news`, `wikipedia_article` |

A scenario fails when the exit status is non-zero, a driver assertion fails, or an expected
substring is missing. Assertions make incorrect scroll/tab/URL state fail immediately. Older
scenarios still use log checks and have narrower evidence; an action log alone does not prove
its outcome. Saved-page coverage now opens the saved fixture and checks its article title.
The keyboard-caret case submits to the local fixture server, so the default suite needs no
live search service. Logs and final frames are retained in the output folder.

The harness creates `KB_TEST_OUT` when supplied. Run against a free fixture port; inspect
`server.log` if startup fails. Current tests do not prove cookie restart persistence, full
offline asset capture, or behavior with Wi-Fi disabled.

### 3.2.4 Unit tests

Run `cargo test --workspace --exclude kb-kindle`. The device crate is excluded because its
input code only runs on the device. Current coverage:

- `kb-net`: tracker blocklist (including subdomains), image detection by extension and
  path segment, rejection of non-HTTP schemes.
- `kb-engine`: title extraction, classic-script rule (ES modules skipped), text-field types,
  encoded fragment IDs/named anchors and absolute scaled scroll positions.
- `kb-app`: search-or-URL normalisation, host and path split, page CSS placed in the head
  (standards mode kept), PNG signature, percent encoding, same-document URL comparison
  (query/origin/path changes require a new document).

The fragment engine test exercises a small fixed layout. General layout and script behavior
are covered by L2; hardware behavior is covered only by L5.

### 3.2.5 Known computer-test limits

- The suite checks behaviour, not pixels. Visual changes (sizes, colours, icons) are checked
  by looking at the frames written to `KB_OUT` and recorded in the change log.
- The desktop engine is faster than the device. Timings on the computer are a lower bound.
- Live-site results depend on the sites, which change.

### 3.2.6 Test pipeline and artifacts

`scripts/test-all.sh` runs every computer check in order and keeps the evidence in the
repository, so a reviewer can see what was tested without rebuilding anything:

| Path | Contents |
|---|---|
| `artifacts/REPORT.md` | One row per step: build, unit tests, desktop regression, launcher tests, Kindle build; each with PASS, FAIL or SKIP and a link to its log |
| `artifacts/build.log`, `artifacts/unit/unit-tests.log` | Workspace build and unit test output |
| `artifacts/desktop/INDEX.md` | One row per regression scenario: result, final screen, named shots |
| `artifacts/desktop/<scenario>/` | `script.txt` (the commands), `log.txt` (driver output), `final.png` (screen at the end), and named shots from `shot` commands |
| `artifacts/desktop/gallery_pages/` | Named full-screen shots of every page and sheet: home, bookmarks, saved pages, menu, Display, tabs, reader, error page, keyboard |
| `artifacts/launcher/launcher-stub-test.log` | Launcher checks |
| `artifacts/device/arm-build.log` | Kindle cross-build: binary size, and whether the copy on the mounted Kindle matches |

Run `scripts/test-all.sh` (add `--network` for the live-site checks). The exit status is 0 only
when every required step passes. The Kindle build is reported as SKIP, not FAIL, when
`cargo-zigbuild` is not installed.

Artifacts are regenerated on every run and committed with the change that produced them. Each
run replaces the previous scenario folders, so `git diff` on `artifacts/` shows what changed on
screen. A screenshot that changed without a matching code change is a regression to investigate.

The screenshots are desktop renders. They show layout, text and state, not the e-ink look; the
device checklist in §3.5 still covers how the screen looks on the panel.

## 3.3 Launcher tests

`scripts/launcher-stub-test.sh` stubs `start`, `stop`, `lipc-set-prop`, and the browser.
Its ten checks cover:

1. Default mode hides/restores the status bar, opens the library, and removes its marker.
2. A killed launcher is recovered by the watchdog, with the same UI/library restoration.
3. A stale `keep` marker is restored before a new run; the run finishes within 30 seconds.
4. The `stop-framework` flag stops/restarts the framework and clears its marker.
5. An empty marker from an older launcher restores the framework first.

These checks prove stubbed call order and cleanup. Recorded hardware observations are in
[device-notes.md](device-notes.md); they complement the complete release checklist in §3.5.

## 3.4 Build and install checks

Before a device test, the build must pass in this order:

1. `cargo build --release --workspace` (no warnings from the browser crates);
2. `cargo test --workspace --exclude kb-kindle`;
3. `scripts/desktop-regression.sh` (82 of 82);
4. `cargo zigbuild --release -p kb-kindle --target armv7-unknown-linux-musleabihf`;
5. `file target/armv7-unknown-linux-musleabihf/release/kb-kindle` reports
   `ELF 32-bit LSB executable, ARM, EABI5` and `statically linked`;
6. `scripts/launcher-stub-test.sh`;
7. `sh deploy/install.sh` reports the binary size and the start page.

## 3.5 Device validation checklist (L5)

Record each item as PASS, FAIL or NOT RUN, with the evidence named. Items marked **must**
block a release.

### Setup

- **D-0 (must)** The binary and launcher on the device match the build: `deploy/install.sh`
  output, and `diff` of the launcher. Evidence: the install output.

### Display and touch

- **D-1 (must)** The home screen appears after launch, with the bookmark tiles.
  Evidence: `browser/frame.png`, the first `ready` line in the log.
- **D-2 (must)** Tapping the bookmark tile opens the page. Evidence: log `open` line with the
  page title.
- **D-3 (must)** On a long article page, the mobile-only "continue reading" control is shown
  and tapping it does what it should. Evidence: photo of the screen; log `tap` lines.
- **D-4 (must)** A tap on a link navigates; a tap on a button that calls JavaScript does not
  navigate. Evidence: log lines.
- **D-5 (must)** Scrolling a page moves the content in the direction of the swipe, and the
  bars hide and reappear. Evidence: log `scroll` lines, photo.
- **D-6 (must)** The menu sheet scrolls by swiping on it, and the page underneath does not
  move. Evidence: photo.
- **D-7** The keyboard appears at the bottom; typing inserts at the caret; Go loads the
  address. Evidence: photo of the field with the caret.
- **D-8** Long press on a link opens it in a new tab. A horizontal swipe on the address bar
  switches tabs. Evidence: log lines.
- **D-9 (must)** Exiting from the menu returns to the Kindle library, with the UI fully
  restored. Evidence: photo of the library; logs for status-bar restoration (or framework restoration
  in fallback mode) and `opened library home`.
- **D-10 (must)** After a forced kill of the browser (pick a method the owner is comfortable
  with), the UI is restored within 10 seconds. Evidence: photo, log.
- **D-11** The page-turn buttons scroll one screen. Evidence: log `page` lines.
- **D-12 (must)** Read two long pages at different positions, alternate Back/Forward, then
  open/switch/close a tab. Each page returns to the same position. Closing the final tab
  leaves one usable Home tab. Evidence: action logs and photos of the restored paragraphs.
- **D-13 (must)** Open a long menu and enough tabs to scroll the switcher. Done and Exit
  remain visible and tappable after scrolling. Evidence: photos and sheet action logs.
- **D-14** Follow a chapter/section link (including an encoded Vietnamese ID), use Back/Forward,
  then reload a URL with a fragment. Evidence: destination headings and address in logs.
- **D-15** Change Narrow/Medium/Wide on a responsive page; confirm the expected layout and
  readable controls, then repeat in reading view and Night mode (standalone) or dark mode
  (under Primitive Launcher). Evidence: photos.
- **D-16** Saved fixture opens with the network unavailable; identify any missing assets
  separately from the saved HTML. Evidence: photo, Wi-Fi state, and logs.

### Under Primitive Launcher (the system layer)

- **D-17 (must)** Press the power button: the lock screen is the launcher's sleep picture.
  Wake: the browser's screen comes back as it was, without a restart. Evidence: photo; no
  second `start` line in the log.
- **D-18 (must)** Swipe down from the top edge: the launcher's quick settings open over the
  page, not the browser's own panel. Close them: the page comes back without a full flash.
  Evidence: photo, log.
- **D-19 (must)** Switch dark mode in the launcher's quick settings over a page: one flash
  when the theme changes, then the whole browser (bars and page) in the new theme with no
  second flash. The Display sheet has no Day and Night. Evidence: photos; `redraw requested by
  the launcher` in the log.
- **D-20** Tap Power in quick settings, then Cancel: the power menu is drawn over the browser,
  and Cancel returns to it. Evidence: photo.

### Performance (record numbers)

- **P-1** Tap feedback: 20 taps; record the time from the touch to the highlight (camera
  timing or log `touch down` to `frame in`). Target NFR-1.
- **P-2** Chrome change (open and close the menu, toggle bars): 10 samples of `action` plus
  `frame` times from the log. Target NFR-2.
- **P-3** Scroll: 10 samples on a long article. Target NFR-3.
- **P-4** First screen of the bookmarked page and of Hacker News: time from `open` to
  `ready`. Target NFR-4.

### Memory and durability

- **M-1** Resident memory: the `rss` value in the action log after opening 10 pages in one
  session. Target NFR-5 (under 250 MB peak).
- **M-2** A page with a looping script (write a test page with `while (true) {}` in a
  script): the page is interrupted within 3 seconds and the browser keeps running. NFR-8.
- **M-3** Cookies: set a cookie on a test site, restart the browser, confirm the cookie is
  sent. FR-2.
- **M-4** Saved page opens with Wi-Fi off. NFR-12.
- **M-5** The e-ink panel shows no visible full-screen flash on routine updates, except the
  periodic clearing flash. NFR-11 (visual).

### Pass criteria

- Every **must** item passes.
- P-1 median under 150 ms, and P-2 median under 400 ms, to meet NFR-1 and NFR-2.
- M-1 peak under 250 MB.
- Any item that fails is recorded in the change log with its evidence.

## 3.6 Evidence to keep

For each device test run, keep under `docs/evidence/<date>/` (not committed when it contains
photos of personal reading material):

- `log-browser.txt` as copied from the Kindle;
- `frame.png` (last frame);
- `probe.txt` when the stock-keyboard question is being checked;
- the checklist with PASS, FAIL or NOT RUN per item, the device firmware, and the commit hash
  of the build.

## 3.7 Reporting

A test run report states: the commit, the command lines used, the pass count per level, the
failing items with evidence, and the open questions. Failures are never hidden by lowering a
target; a target is changed only with a recorded reason in the change log.

## 3.8 Known gaps in the pipeline

- No automated test drives the touch panel or the e-ink panel; both are checked by hand.
- No automated performance test on the device; the log supplies the numbers manually.
- No cross-check against a second browser engine for layout or script behaviour.
- The fixtures cover common paths; sites with unusual markup are only covered by L3 smoke
  tests and the device checklist.


## 3.9 Navigation milestone validation — 2026-11-16

Baseline commit: `e8d7d74`; results cover the working-tree changes described in Unreleased.
Commands were run with Homebrew Rust/zig on PATH.

| Check | Result |
|---|---|
| `cargo test --workspace` | PASS, 64 unit tests (kb-app 33, kb-term 12, kb-net 7, kb-kindle 6, kb-engine 5, kb-js 1) |
| `cargo build --release --workspace` | PASS; host `rust-objcopy` warns that debug stripping could not load `libLLVM.dylib` |
| `scripts/desktop-regression.sh` | PASS, 86/86 local scenarios (2026-11-17) |
| `scripts/launcher-stub-test.sh` | PASS, ten checks; intentional killed-launcher messages expected |
| ARMv7 `cargo zigbuild` from §3.4 | PASS; vendored QuickJS emits four unused-variable warnings |
| `file target/armv7-unknown-linux-musleabihf/release/kb-kindle` | ELF 32-bit ARM EABI5, statically linked, stripped |
| Visual review | Desktop frames: pinned Menu/Tabs headers, close-last-tab Home state; Day/Night display and scrolled bodies reviewed |
| Live-site suite | NOT RUN in this milestone |
| Install/device checklist/performance | NOT RUN; no Kindle mounted during this run |

The initial unit-test baseline failed to compile because two Settings fixtures omitted new
fields. They now use a complete default. The local baseline had 29 passing scenarios; 13
navigation/UI cases were added. Test artifacts were written to `/tmp/kb-navigation-commit-check`;
that path is local and temporary, not durable device evidence. The stale-marker case now
waits for its recovery run to exit before deleting temporary files, and verifies that its marker was cleared. Intentional killed-launcher output belongs
to the watchdog scenario.
