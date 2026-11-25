#!/usr/bin/env python3
"""Automated tests on the Kindle: Primitive Browser driven by recorded touch and button input.

The browser reads its touch and page buttons from FIFOs named in two files of its folder
(`test-touch`, `test-buttons`). This script creates those FIFOs on the Kindle, starts the browser, and
writes real `input_event` records into them: taps, long presses, swipes, page-button presses, and
timed sequences. The browser's own gesture code turns the records into events. The results are read
from the browser's log on the Kindle (`log-browser.txt`).

Needs: the Kindle on Wi-Fi with its SSH server running, and the root password given through SSH_ASKPASS
(an askpass program that prints it; keep the password outside this repository). Nothing secret is stored here.

  SSH_ASKPASS=/path/to/askpass SSH_ASKPASS_REQUIRE=force DISPLAY=:0 tests/device/run.py [--host IP] [--port N]

The browser on the Kindle must be the build with the test input (version 0.2.23 or later).
"""

import argparse
import os
import random
import re
import struct
import subprocess
import sys
import time

# Linux input constants (the Kindle's kernel).
EV_SYN, EV_KEY, EV_ABS = 0x00, 0x01, 0x03
SYN_REPORT = 0x00
ABS_MT_POSITION_X, ABS_MT_POSITION_Y, ABS_MT_TRACKING_ID = 0x35, 0x36, 0x39
KEY_PAGEUP, KEY_PAGEDOWN = 104, 109
RECORD = "<iiHHi"  # tv_sec, tv_usec, type, code, value (32-bit kernel)

BROWSER_DIR = "/mnt/us/browser"
LOG = f"{BROWSER_DIR}/log-browser.txt"
START = "/mnt/us/documents/Primitive Browser.sh"
FIFO_TOUCH = "/var/tmp/kb-test-touch"
FIFO_BUTTONS = "/var/tmp/kb-test-buttons"

# Screen positions (device pixels, portrait 1264 x 1680), measured with the desktop driver on the same layout.
MENU = (1100, 1628)  # bottom bar: the menu
TABS = (670, 1628)  # bottom bar: the tabs
OUT_DIR = None  # set by --out
ABOVE_SHEET = (600, 300)  # a tap above an open sheet dismisses it
EXIT_CONTROL = (1200, 920)  # the menu's Exit, in its header


class Kindle:
    def __init__(self, host, port):
        self.host, self.port = host, port
        self.ctl = f"/tmp/kb-test-ctl-{os.getpid()}"
        self.opts = ["-p", str(port), "-o", "StrictHostKeyChecking=accept-new",
                     "-o", "ControlMaster=auto", "-o", f"ControlPath={self.ctl}", "-o", "ControlPersist=600"]

    def run(self, command, data=None, timeout=60):
        """Run a shell command on the Kindle and return its stdout (text)."""
        result = subprocess.run(["ssh", *self.opts, f"root@{self.host}", command],
                                input=data, capture_output=True, timeout=timeout)
        return result.stdout.decode("utf-8", "replace")

    def stream(self, command):
        """Start a command on the Kindle with a pipe to it (its stdin); used to write the input records."""
        return subprocess.Popen(["ssh", *self.opts, f"root@{self.host}", command],
                                stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def close(self):
        subprocess.run(["ssh", "-p", str(self.port), "-o", f"ControlPath={self.ctl}", "-O", "exit",
                        f"root@{self.host}"], capture_output=True)

    def log_size(self):
        return int(self.run(f"wc -c < '{LOG}' 2>/dev/null || echo 0").strip() or 0)

    def log_since(self, offset):
        return self.run(f"tail -c +{offset + 1} '{LOG}' 2>/dev/null")


class Pen:
    """Writes input records into one FIFO stream. Each call flushes, so the browser sees the event now."""

    def __init__(self, stream, touch_id=[0]):
        self.stream = stream
        self.touch_id = touch_id

    def record(self, kind, code, value):
        now = time.time()
        self.stream.stdin.write(struct.pack(RECORD, int(now), int((now % 1) * 1_000_000), kind, code, value))
        self.stream.stdin.flush()

    def sync(self):
        self.record(EV_SYN, SYN_REPORT, 0)

    def down(self, x, y):
        self.touch_id[0] += 1
        self.record(EV_ABS, ABS_MT_TRACKING_ID, self.touch_id[0])
        self.record(EV_ABS, ABS_MT_POSITION_X, int(x))
        self.record(EV_ABS, ABS_MT_POSITION_Y, int(y))
        self.sync()

    def move(self, x, y):
        self.record(EV_ABS, ABS_MT_POSITION_X, int(x))
        self.record(EV_ABS, ABS_MT_POSITION_Y, int(y))
        self.sync()

    def up(self):
        self.record(EV_ABS, ABS_MT_TRACKING_ID, -1)
        self.sync()

    def tap(self, x, y, hold=0.06):
        self.down(x, y)
        time.sleep(hold)
        self.up()

    def long_tap(self, x, y):
        self.tap(x, y, hold=1.2)

    def swipe(self, a, b, duration=0.35, steps=12):
        (x0, y0), (x1, y1) = a, b
        self.down(x0, y0)
        for i in range(1, steps + 1):
            time.sleep(duration / steps)
            self.move(x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps)
        self.up()

    def button(self, code, hold=0.06):
        self.record(EV_KEY, code, 1)
        self.sync()
        time.sleep(hold)
        self.record(EV_KEY, code, 0)
        self.sync()


def screenshot(kindle, path):
    """Save the Kindle's framebuffer (Y8, 1280-byte lines; the panel is 1264 x 1680) as a PNG."""
    import zlib
    raw = subprocess.run(["ssh", *kindle.opts, f"root@{kindle.host}", "dd if=/dev/fb0 bs=1280 count=1680 2>/dev/null"],
                         capture_output=True, timeout=120).stdout
    width, height, stride = 1264, 1680, 1280
    if len(raw) < stride * height:
        return False
    data = b"".join(b"\x00" + raw[y * stride:y * stride + width] for y in range(height))

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(data, 6)) + chunk(b"IEND", b""))
    with open(path, "wb") as f:
        f.write(png)
    return True


class Report:
    def __init__(self):
        self.results = []

    def check(self, name, ok, detail=""):
        self.results.append((name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  [{detail}]" if detail else ""), flush=True)

    def info(self, name, detail):
        self.results.append((name, None, detail))
        print(f"INFO  {name}  [{detail}]", flush=True)

    def summary(self):
        failed = [r for r in self.results if r[1] is False]
        print(f"\n{len(self.results) - len(failed)} passed, {len(failed)} failed")
        return 1 if failed else 0


def wait_for(kindle, offset, pattern, timeout=12.0):
    """Poll the browser log (from `offset`) until a line matches `pattern`. Returns the matching text or None."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        text = kindle.log_since(offset)
        match = re.search(pattern, text)
        if match:
            return text
        time.sleep(0.4)
    return None


def start_browser(kindle, report):
    """Stop any running browser, create the input FIFOs, start the browser, and attach the input writers."""
    # Stop any earlier browser and its launcher script (they restart the browser after an exit): TERM first, then KILL
    # what is still there. The patterns are written with brackets so that this command line does not match them itself.
    kindle.run("for sig in TERM KILL; do for p in /proc/[0-9]*; do c=$(tr '\\0' ' ' 2>/dev/null < $p/cmdline); "
               "case \"$c\" in *[k]b-kindle*|*[P]rimitive\\ Browser.sh*) kill -$sig ${p#/proc/} 2>/dev/null;; esac; done; sleep 2; done")
    kindle.run(f"printf '{FIFO_TOUCH}' > {BROWSER_DIR}/test-touch; printf '{FIFO_BUTTONS}' > {BROWSER_DIR}/test-buttons; "
               f"rm -f {FIFO_TOUCH} {FIFO_BUTTONS}; mkfifo {FIFO_TOUCH} {FIFO_BUTTONS}")
    offset = kindle.log_size()
    kindle.run(f"cd {BROWSER_DIR} && PRIMITIVE_LAUNCHER=1 setsid sh '{START}' >/dev/null 2>&1 </dev/null &")
    # The browser's input threads open their FIFOs for reading and wait there for a writer, so the writers are
    # attached first (a FIFO open for writing meets its reader); the browser logs its input once both are open.
    touch = kindle.stream(f"cat > {FIFO_TOUCH}")
    buttons = kindle.stream(f"cat > {FIFO_BUTTONS}")
    text = wait_for(kindle, offset, r"buttons /\S+ open \(test input\)", timeout=40)
    report.check("browser starts and reads the test input", text is not None)
    if text is None:
        raise SystemExit("the browser did not open the test input; see the Kindle log")
    time.sleep(1.0)
    wait_for(kindle, offset, r"ready in", timeout=40)
    return offset, Pen(touch), Pen(buttons)


def events_in(text):
    return re.findall(r"\[kb\] event (\w+)", text)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default=os.environ.get("KINDLE_HOST", "192.168.1.28"))
    ap.add_argument("--port", type=int, default=int(os.environ.get("KINDLE_SSH_PORT", "2222")))
    ap.add_argument("--out", default="", help="folder for the screenshot and the log of the run")
    args = ap.parse_args()
    global OUT_DIR
    if args.out:
        os.makedirs(args.out, exist_ok=True)
        OUT_DIR = args.out
    if not os.environ.get("SSH_ASKPASS"):
        sys.exit("set SSH_ASKPASS (and SSH_ASKPASS_REQUIRE=force, DISPLAY=:0) to an askpass that prints the password")

    kindle = Kindle(args.host, args.port)
    report = Report()
    try:
        offset, touch, buttons = start_browser(kindle, report)
        log = kindle.log_since(offset)
        report.check("the touch input is the test stream", "touch /dev/" not in log and "(test input)" in log)
        version = re.search(r"start v(\S+)", log)
        report.info("browser version", version.group(1) if version else "not in the log")

        # 1. Swipe up scrolls the page (and hides the bars).
        mark = kindle.log_size()
        touch.swipe((632, 1200), (632, 600))
        text = wait_for(kindle, mark, r"event Scroll\(")
        report.check("swipe up scrolls the page", text is not None)

        # 2. The page buttons turn pages (forward and back).
        mark = kindle.log_size()
        buttons.button(KEY_PAGEDOWN)
        text = wait_for(kindle, mark, r"event Page\(1\.0\)")
        report.check("page-forward button turns the page", text is not None)
        mark = kindle.log_size()
        buttons.button(KEY_PAGEUP)
        text = wait_for(kindle, mark, r"event Page\(-1\.0\)")
        report.check("page-back button turns the page back", text is not None)
        time.sleep(1.0)

        # 3. Bottom bar: the menu opens; a tap above it closes it. A swipe down shows the bars again (a page turn
        #    forward hid them, and a tap on the page would not be a bar tap).
        touch.swipe((632, 600), (632, 720))
        time.sleep(1.0)
        mark = kindle.log_size()
        touch.tap(*MENU)
        text = wait_for(kindle, mark, r"action kb:menu")
        report.check("tapping the menu button opens the menu", text is not None)
        time.sleep(1.0)
        mark = kindle.log_size()
        touch.tap(*ABOVE_SHEET)
        text = wait_for(kindle, mark, r"dismissed by tap above sheet")
        report.check("a tap above the menu closes it", text is not None)
        time.sleep(0.8)

        # 4. The tabs sheet opens from the bottom bar, and closes the same way.
        touch.swipe((632, 600), (632, 720))
        time.sleep(1.0)
        mark = kindle.log_size()
        touch.tap(*TABS)
        text = wait_for(kindle, mark, r"action kb:tabs")
        report.check("tapping the tabs button opens the tabs sheet", text is not None)
        time.sleep(1.0)
        mark = kindle.log_size()
        touch.tap(*ABOVE_SHEET)
        time.sleep(0.8)

        # 6. Horizontal swipes: back and forward; a swipe along the top bar steps tabs.
        mark = kindle.log_size()
        touch.swipe((150, 900), (700, 900))
        text = wait_for(kindle, mark, r"event Back")
        report.check("swipe right is Back", text is not None)
        time.sleep(0.8)
        mark = kindle.log_size()
        touch.swipe((900, 40), (300, 40))
        text = wait_for(kindle, mark, r"event TabStep")
        report.check("swipe along the top bar steps tabs", text is not None)
        time.sleep(0.8)

        # 7. A long press is a separate event from a tap.
        mark = kindle.log_size()
        touch.long_tap(632, 900)
        text = wait_for(kindle, mark, r"event LongTap")
        report.check("a long press is a long-tap event", text is not None)
        time.sleep(1.0)

        # 8. The key check: bars stay responsive while a page is painted.
        #    A short swipe down shows the bars and moves the page to a new place: the picture must be painted
        #    (about 300 ms on the Oasis 3; a screen that was prefetched would need no paint, so the check says so).
        #    A tap on the menu sent 40 ms later must be handled before that paint finishes.
        # A page-forward press first moves the page away from the top (it hides the bars), so the swipe below lands on
        # a new scroll position: a position seen before is cached, and at the top a scroll up is clamped to position 0.
        buttons.button(KEY_PAGEDOWN)
        wait_for(kindle, kindle.log_size() - 1, r"page ready|event Page", timeout=4)
        time.sleep(1.0)
        mark = kindle.log_size()
        # A long swipe (450 to 650 px) reaches a place the browser has not drawn, so it needs a paint.
        distance = random.randint(450, 650)
        print(f"  bars check: swipe down {distance} px", flush=True)
        touch.swipe((632, 300), (632, 300 + distance), duration=0.3, steps=8)
        time.sleep(0.04)
        touch.tap(*MENU)
        wait_for(kindle, mark, r"page ready", timeout=8)
        time.sleep(0.8)
        log = kindle.log_since(mark)
        verdict = bar_responsive_verdict(log)
        report.check("bars answer while a page is painted", verdict[0] is True, verdict[1])
        time.sleep(0.8)
        mark = kindle.log_size()
        touch.tap(*ABOVE_SHEET)
        time.sleep(0.8)

        # 9. Rapid mixed input: turns and bar taps in turn. The browser must stay alive and log no crash.
        mark = kindle.log_size()
        for i in range(20):
            if i % 3 == 0:
                buttons.button(KEY_PAGEDOWN if i % 2 == 0 else KEY_PAGEUP, hold=0.02)
            elif i % 3 == 1:
                touch.tap(*MENU, hold=0.02)
            else:
                touch.tap(*ABOVE_SHEET, hold=0.02)
            time.sleep(0.08)
        time.sleep(3.0)
        log = kindle.log_since(mark)
        alive = "kb-kindle" in kindle.run("for p in /proc/[0-9]*; do tr '\\0' ' ' 2>/dev/null < $p/cmdline; echo; done | grep kb-kindle | grep -v grep")
        report.check("rapid input leaves the browser running", alive)
        report.check("rapid input logs no crash", "[crash]" not in log and "recovered from a panic" not in log)

        # 10. Memory: resident size from the action lines.
        rss = [int(m) for m in re.findall(r"rss (\d+) kB", kindle.log_since(offset))]
        if rss:
            report.info("resident memory (kB), first and last action", f"{rss[0]} -> {rss[-1]}, max {max(rss)}")

        # 11. The screen as the reader last saw it (kept with the results).
        if OUT_DIR:
            shot = screenshot(kindle, os.path.join(OUT_DIR, "screen-after-input.png"))
            report.info("screenshot", "screen-after-input.png" if shot else "the framebuffer could not be read")

        # 11b. No crash in the whole run.
        log = kindle.log_since(offset)
        report.check("no crash in the whole run", "[crash]" not in log and "recovered from a panic" not in log)

        # 12. Clean exit through the menu's Exit: the browser logs "bye". (The keyboard is not opened here: its
        #     cancel control is not part of the device run; the keyboard is covered by the desktop regression.)
        touch.swipe((632, 600), (632, 720))
        time.sleep(1.0)
        mark = kindle.log_size()
        touch.tap(*MENU)
        time.sleep(1.0)
        touch.tap(*EXIT_CONTROL)
        text = wait_for(kindle, mark, r"\[kb\] bye", timeout=10)
        report.check("Exit closes the browser cleanly", text is not None)
    finally:
        # Leave the Kindle as the user had it: the test input files are removed (the browser would wait for them),
        # and the launcher is asked to redraw its home screen (the browser was started outside the launcher, so
        # nothing else would draw it after the test's Exit).
        try:
            kindle.run(f"rm -f {BROWSER_DIR}/test-touch {BROWSER_DIR}/test-buttons; "
                       "kill -USR1 $(pidof kl-launcher) 2>/dev/null; true", timeout=30)
        except subprocess.TimeoutExpired:
            print("WARNING  the Kindle did not answer the cleanup: remove test-touch and test-buttons by hand", flush=True)
        if OUT_DIR:
            try:
                with open(os.path.join(OUT_DIR, "browser-log.txt"), "w") as f:
                    f.write(kindle.log_since(max(0, kindle.log_size() - 200000)))
            except subprocess.TimeoutExpired:
                print("WARNING  the browser log could not be saved", flush=True)
        kindle.close()
    return report.summary()


def bar_responsive_verdict(log):
    """Judge one page move (a swipe or a page turn) with a tap on the menu sent 40 ms after it.

    The move's own log line must say a paint was in flight (so the picture really was being painted); the menu's
    action must then come before "page ready" (the paint's end). Otherwise the check is inconclusive and fails.
    """
    lines = log.splitlines()
    turn = next((i for i, l in enumerate(lines) if re.search(r"event (Page|Scroll)\(", l)), None)
    if turn is None:
        return False, "the page move was not logged"
    if not any("page paint in flight" in l for l in lines[turn:turn + 2]):
        return False, "the move did not start a paint (nothing to overlap)"
    tap = next((i for i, l in enumerate(lines) if i > turn and "event Tap(1100" in l), None)
    if tap is None:
        return False, "the menu tap was not logged"
    action = next((i for i, l in enumerate(lines) if i > tap and "[ui] action kb:menu" in l), None)
    if action is None:
        return False, "the menu tap was not handled as a bar tap"
    ready = next((i for i, l in enumerate(lines) if i > turn and "page ready" in l), None)
    if ready is None or action < ready:
        return True, f"menu handled at log line {action}, paint still running (page ready at {ready})"
    return False, f"page ready at line {ready} came before the menu at {action}"


if __name__ == "__main__":
    sys.exit(main())
