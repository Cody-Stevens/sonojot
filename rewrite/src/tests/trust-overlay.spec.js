// The inline doubtful-words overlay with a fake native side: views arrive as
// "trust-view" events, everything the page reports is recorded, and audio
// playback is counted instead of heard. Made-up text only.
import { expect, test } from "@playwright/test";

function view(patch = {}) {
  return {
    session: 1,
    scale: 1,
    area: { x: 0, y: 0, w: 1220, h: 860 },
    hostDark: false,
    markStyle: "band",
    popoverStyle: "card",
    playOn: "hover",
    dictationKey: "Ctrl+Space",
    hold: false,
    marks: [
      {
        id: 1,
        kind: "word",
        heard: "Castrel",
        alternatives: [{ text: "Kestrel", label: "Your words" }],
        hasAudio: true,
        rects: [{ x: 240, y: 200, w: 68, h: 22 }],
      },
      {
        id: 2,
        kind: "gap",
        heard: "strokes",
        alternatives: [],
        hasAudio: true,
        rects: [{ x: 380, y: 200, w: 14, h: 22 }],
      },
    ],
    focus: null,
    voice: null,
    message: null,
    announce: null,
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
  await expect(page.locator(".mk").first()).toBeVisible();
}

const sentTypes = (page) => page.evaluate(() => window.sent.map((e) => e.type));

test("no fallback chip or panel ever renders", async ({ page }) => {
  await open(page);
  await expect(page.locator(".chip")).toHaveCount(0);
  await expect(page.locator(".fbp")).toHaveCount(0);
});

test("clicking a mark opens the card, explains it, and plays audio once", async ({
  page,
}) => {
  await open(page);
  await page.locator(".mk").first().click();
  await expect(page.locator(".pop")).toBeVisible();
  await expect(page.locator(".pop")).toContainText("Unsure about");
  await expect(page.locator(".pop")).toContainText("Castrel");
  expect(await sentTypes(page)).toContain("opened");
  await expect.poll(() => page.evaluate(() => window.played)).toBe(1);
  // Pointer mode shows symbols rather than keyboard keycaps so users don't type into their host app.
  await expect(page.locator(".pop .opt .n").first()).toHaveText("→");
  await expect(page.locator(".pop .opt.keep .n")).toHaveText("✓");
  // Clicking the waveform stops playback while playing, then replays on next click.
  await page.locator(".wave-btn").click();
  await page.locator(".wave-btn").click();
  await expect.poll(() => page.evaluate(() => window.played)).toBe(2);
});

test("no audio plays by itself while recording or with On click", async ({
  page,
}) => {
  await open(page, { busy: true });
  await page.locator(".mk").first().click();
  await expect(page.locator(".pop")).toBeVisible();
  await page.waitForTimeout(250);
  expect(await page.evaluate(() => window.played)).toBe(0);
  await page.locator(".pop .play").click();
  await expect.poll(() => page.evaluate(() => window.played)).toBe(1);
});

test("On click plays nothing on open", async ({ page }) => {
  await open(page, { playOn: "click" });
  await page.locator(".mk").first().click();
  await expect(page.locator(".pop")).toBeVisible();
  await page.waitForTimeout(250);
  expect(await page.evaluate(() => window.played)).toBe(0);
});

test("a mark without alternatives has no stray divider and guides voice or typing", async ({
  page,
}) => {
  await open(page);
  await page.locator(".gapmk").click();
  await expect(page.locator(".pop")).toBeVisible();
  await expect(page.locator(".pop")).toContainText(
    "Possible missing word after",
  );
  await expect(page.locator(".pop .sep")).toHaveCount(0);
  await expect(page.locator(".pop .foot")).toContainText("type over it");
});

test("Keep all dismisses every remaining mark in one click", async ({
  page,
}) => {
  await open(page);
  await page.locator(".mk").first().click();
  await page.getByRole("button", { name: "Keep all" }).click();
  await expect(page.locator(".pop")).toHaveCount(0);
  expect(await sentTypes(page)).toContain("keepAll");
});

test("keyboard focus shows 1/K shortcuts and handles K and Escape", async ({
  page,
}) => {
  await open(page, { focus: { text: "1", seq: 1 } });
  await expect(page.locator(".pop")).toBeVisible();
  await expect(page.locator(".pop .opt").first()).toBeFocused();
  await expect(page.locator(".pop .opt .n").first()).toHaveText("1");
  await expect(page.locator(".pop .opt.keep .n")).toHaveText("K");
  await page.keyboard.press("k");
  const keep = await page.evaluate(() =>
    window.sent.find((e) => e.type === "keep"),
  );
  expect(keep).toEqual({ type: "keep", mark: 1 });

  await page.evaluate(
    (v) => window.emitView(v),
    view({ focus: { text: "2", seq: 2 } }),
  );
  await expect(page.locator(".pop")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".pop")).toHaveCount(0);
  const closed = await page.evaluate(() =>
    window.sent.find((e) => e.type === "closed"),
  );
  expect(closed).toEqual({ type: "closed", refocus: true });
});

test("strip mode can always be closed after clicking a mark", async ({
  page,
}) => {
  await open(page, { popoverStyle: "strip" });
  await page.locator(".mk").first().click();
  await expect(page.locator(".pop.strip")).toBeVisible();
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.locator(".pop.strip")).toHaveCount(0);
});

test("popover closes quietly when its mark scrolls out of view", async ({
  page,
}) => {
  await open(page);
  await page.locator(".mk").first().click();
  await expect(page.locator(".pop")).toBeVisible();
  const scrolled = view();
  scrolled.marks[0].rects = [];
  await page.evaluate((v) => window.emitView(v), scrolled);
  await expect(page.locator(".pop")).toHaveCount(0);
});
