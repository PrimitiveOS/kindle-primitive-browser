# 7. Device tests

Automated tests on the Kindle Oasis 3. They drive the browser with the same input records the touch screen and the
page buttons produce, then read the browser's own log. Run them after a device build, before a release.

## 7.1 What is tested

`tests/device/run.py` checks, on the device:

- scrolling by a swipe up, and the bars coming back on a swipe down;
- the page buttons turn pages forward and back;
- the menu and tabs buttons open their sheets; a tap above a sheet closes it;
- swipes: right is Back, a swipe along the top bar steps tabs;
- a long press is a long-tap event;
- the bars answer while a page is painted: a paint is started by a swipe, and the menu tap sent 40 ms later is
  handled before the paint finishes;
- rapid mixed input (turns and bar taps) leaves the browser running and logs no crash;
- Exit from the menu closes the browser cleanly.

The keyboard is not part of the device run (its cancel control has no automated position yet); the desktop
regression covers it (`scripts/desktop-regression.sh`).

## 7.2 How input is injected

The browser reads its touch and page buttons from the device's input nodes. A file in the browser folder changes
that: `test-touch` and `test-buttons` each hold the path of a FIFO. When a file is present, the browser reads
`input_event` records from that FIFO instead of the device node, and the gesture code is unchanged. The files are
absent by default, so a normal start uses the real touch and buttons. The run removes them when it ends.

## 7.3 Running

Needs the Kindle on Wi-Fi with its SSH server running. The password is given through `SSH_ASKPASS`, an askpass
program outside the repository; nothing secret is stored here.

    SSH_ASKPASS=/path/to/askpass SSH_ASKPASS_REQUIRE=force DISPLAY=:0 \
        python3 tests/device/run.py --out /path/to/results

The results folder gets `browser-log.txt` (the browser's log of the run) and `screen-after-input.png` (the
framebuffer before Exit). The browser is started outside the launcher, so the run asks the launcher to redraw its
home screen at the end. If the screen stays blank after a run, send `kill -USR1` to `kl-launcher` on the Kindle.

## 7.4 Limits

- Taps on the page content wait for a paint in progress (see [06-page-worker.md](06-page-worker.md), 6.5). The
  test does not time that wait.
- The screen is read from the framebuffer as a picture; the run does not compare pixels, only the browser's log
  and the screenshot for a person to look at.
