// The doubtful-words chip and panel with a fake native side: views arrive as
// "trust-view" events, everything the page reports is recorded, and audio
// playback is counted instead of heard. Made-up text only.
import { expect, test } from "@playwright/test";

const text = "Send the draft with broad strokes, style and colour notes.";
const gapAt = text.indexOf(",");

function view(patch = {}) {
  return {
    session: 1,
    mode: "chip",
    canReplace: false,
    retype: false,
    scale: 1,
    area: { x: 0, y: 0, w: 1220, h: 860 },
    hostDark: false,
    markStyle: "band",
    popoverStyle: "card",
    playOn: "hover",
    dictationKey: "Ctrl+Space",
    hold: false,
    text,
    marks: [
      {
        id: 1,
        kind: "gap",
        heard: "strokes",
        alternatives: [],
        hasAudio: true,
        rects: [],
        from: gapAt,
        to: gapAt,
      },
    ],
    chip: { x: 400, y: 500, w: 1, h: 18 },
    focus: null,
    voice: null,
    message: null,
    announce: null,
    nearText: false,
    busy: false,
    ...patch,
  };
}

async function open(page, patch) {
  await page.addInitScript(() => {
    window.sent = [];
    window.played = 0;
    const callbacks = new Map();
    let next = 1;
    let handler = null;
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = {
      metadata: {
        currentWindow: { label: "trust" },
        currentWebview: { label: "trust", windowLabel: "trust" },
      },
      transformCallback: (callback) => {
        callbacks.set(next, callback);
        return next++;
      },
      invoke: async (command, args = {}) => {
        if (command === "plugin:event|listen") {
          handler = args.handler;
          return handler;
        }
        if (command === "trust_ui") {
          window.sent.push(args.event);
          return null;
        }
        if (command === "trust_snippet") {
          const samples = 16000;
          const bytes = new Uint8Array(samples * 2);
          return {
            pcm: btoa(String.fromCharCode(...bytes.slice(0, 4000))),
            sampleRate: 16000,
            markStart: 0.3,
            markEnd: 0.6,
            seconds: 1,
            peaks: Array.from({ length: 110 }, (_, i) => (i % 7) / 7),
          };
        }
        return null;
      },
    };
    window.emitView = (payload) =>
      callbacks.get(handler)?.({ event: "trust-view", id: 1, payload });
    class FakeContext {
      state = "running";
      currentTime = 0;
      destination = {};
      createBuffer(_, length) {
        return { length, copyToChannel() {} };
      }
      createBufferSource() {
        return {
          connect() {},
          start() {
            window.played += 1;
          },
          stop() {},
        };
      }
      resume() {
        return Promise.resolve();
      }
    }
    window.AudioContext = FakeContext;
  });
  await page.goto("/index.html?view=trust");
  await page.waitForFunction(() => window.sent.some((e) => e.type === "ready"));
  await page.evaluate((v) => window.emitView(v), view(patch));
  await expect(page.locator(".chip")).toBeVisible();
}

const sentTypes = (page) => page.evaluate(() => window.sent.map((e) => e.type));

test("the chip opens the panel, which takes the keyboard and plays once", async ({ page }) => {
  await open(page);
  await page.locator(".chip").click();
  await expect(page.locator(".fbp")).toBeVisible();
  expect(await sentTypes(page)).toContain("opened");
  await expect(page.locator(".fbp .opt").first()).toBeFocused();
  await expect.poll(() => page.evaluate(() => window.played)).toBe(1);
  await page.screenshot({ path: "../artifacts/trust-chip-fix/panel-far.png" });
});

test("no audio plays by itself while recording or with On click", async ({ page }) => {
  await open(page, { busy: true });
  await page.locator(".chip").click();
  await expect(page.locator(".fbp")).toBeVisible();
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => window.played)).toBe(0);
  await page.locator(".fbp .play").click();
  await expect.poll(() => page.evaluate(() => window.played)).toBe(1);
});

test("On click plays nothing on open", async ({ page }) => {
  await open(page, { playOn: "click" });
  await page.locator(".chip").click();
  await expect(page.locator(".fbp")).toBeVisible();
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => window.played)).toBe(0);
});

test("K records Nothing missing and clears the panel and chip", async ({ page }) => {
  await open(page);
  await page.locator(".chip").click();
  await expect(page.locator(".fbp .opt").first()).toBeFocused();
  await page.keyboard.press("k");
  await expect(page.locator(".fbp")).toHaveCount(0);
  await expect(page.locator(".chip")).toHaveCount(0);
  const keep = await page.evaluate(() => window.sent.find((e) => e.type === "keep"));
  expect(keep).toEqual({ type: "keep", mark: 1 });
});

test("clicking Nothing missing does the same", async ({ page }) => {
  await open(page);
  await page.locator(".chip").click();
  await page.getByRole("option", { name: "Nothing missing" }).click();
  await expect(page.locator(".fbp")).toHaveCount(0);
  await expect(page.locator(".chip")).toHaveCount(0);
  expect(await sentTypes(page)).toContain("keep");
});

test("the close button and Escape close the panel and clear the chip", async ({ page }) => {
  await open(page);
  await page.locator(".chip").click();
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.locator(".fbp")).toHaveCount(0);
  await expect(page.locator(".chip")).toHaveCount(0);
  const closed = await page.evaluate(() => window.sent.find((e) => e.type === "closed"));
  expect(closed).toEqual({ type: "closed", refocus: true });

  await page.evaluate((v) => window.emitView(v), view({ session: 2 }));
  await page.locator(".chip").click();
  await expect(page.locator(".fbp .opt").first()).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.locator(".fbp")).toHaveCount(0);
  await expect(page.locator(".chip")).toHaveCount(0);
});

test("hovering the chip keeps it a little longer", async ({ page }) => {
  await open(page);
  await page.locator(".chip").hover();
  expect(await sentTypes(page)).toContain("chipHover");
});

test("next to the text the panel shows the context line, not the dictation", async ({ page }) => {
  await open(page, { nearText: true });
  await page.locator(".chip").click();
  await expect(page.locator(".fbp")).toBeVisible();
  await expect(page.locator(".fbtext")).toHaveCount(0);
  await expect(page.locator(".fbp")).toContainText("Something may be missing after");
  await expect(page.locator(".fbp")).toContainText("“strokes”");
  await page.screenshot({ path: "../artifacts/trust-chip-fix/panel-near.png" });
});

test("a missing-word spot is a caret between the word and its comma, not a box", async ({ page }) => {
  await open(page);
  await page.locator(".chip").click();
  const gap = page.getByRole("button", { name: "Possible missing word after strokes" });
  await expect(gap).toBeVisible();
  const style = await gap.evaluate((el) => {
    const cs = getComputedStyle(el);
    return { outline: cs.outlineStyle, border: cs.borderTopStyle, width: el.getBoundingClientRect().width };
  });
  expect(style.outline).toBe("none");
  expect(style.border).toBe("none");
  expect(style.width).toBeLessThanOrEqual(12);
  const caret = await gap.locator(".g-caret").evaluate((el) => {
    const r = el.getBoundingClientRect();
    return { w: r.width, h: r.height, bg: getComputedStyle(el).backgroundColor };
  });
  expect(caret.w).toBeGreaterThan(1);
  expect(caret.h).toBeGreaterThan(8);
  expect(caret.bg).not.toBe("rgba(0, 0, 0, 0)");
  // The comma starts after the marker: measure the text node right after it.
  const overlap = await gap.evaluate((el) => {
    let node = el.nextSibling;
    while (node && !(node.nodeType === Node.TEXT_NODE && node.textContent.length))
      node = node.nextSibling;
    const range = document.createRange();
    range.setStart(node, 0);
    range.setEnd(node, 1);
    return range.getBoundingClientRect().left - el.getBoundingClientRect().right;
  });
  expect(overlap).toBeGreaterThanOrEqual(0);
  await page.locator(".fbtext").screenshot({ path: "../artifacts/trust-chip-fix/gap-marker.png" });
});
