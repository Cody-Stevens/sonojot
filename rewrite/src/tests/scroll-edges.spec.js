// Every scrolling region shows the paper-pocket edge cues (design bible,
// October 13 scroll edges): a lip at the top while content is above, at the
// bottom while content is below, hidden at each end, never taking input.
// This walks the main screens, finds whatever actually scrolls, and checks it.
import { expect, test } from "@playwright/test";
import {
  goToPage,
  openApp,
  openFold,
  openMeeting,
  showRecentDictations,
} from "./app-helpers.js";
import { macProfileFixture } from "./desktop-fixture.js";

const pages = [
  "Dictation",
  "Meetings",
  "Microphone",
  "Shortcuts",
  "Vocabulary",
  "Appearance",
  "Models",
  "About",
];

/**
 * Tag every rendered element that scrolls now. Also report rendered elements
 * that would scroll with more content (overflow auto or scroll) but have no
 * cue, so a short list cannot hide a missing mechanism.
 */
async function scrollers(page) {
  return page.evaluate(() => {
    const found = [];
    let index = 0;
    for (const element of document.querySelectorAll("body *")) {
      const style = getComputedStyle(element);
      if (!/(auto|scroll)/.test(style.overflowY)) continue;
      // Closed <details> keep a layout box; only rendered scrollers count.
      if (!element.checkVisibility({ visibilityProperty: true })) continue;
      // Content-sized passages grow instead of scrolling.
      if (style.fieldSizing === "content") continue;
      if (element.scrollHeight - element.clientHeight < 2) {
        if (!element.dataset.scrollEdges)
          found.push({
            id: null,
            mode: null,
            name: `${element.tagName.toLowerCase()}.${[...element.classList].join(".")} (not overflowing)`,
          });
        continue;
      }
      const id = `scroll-${index++}`;
      element.setAttribute("data-audit-scroller", id);
      found.push({
        id,
        mode: element.dataset.scrollEdges ?? null,
        name:
          element.getAttribute("aria-label") ||
          `${element.tagName.toLowerCase()}.${[...element.classList].join(".")}`,
      });
    }
    return found;
  });
}

/** Lip opacity, pointer handling and position for one scroller's edge. */
function lip(page, id, which) {
  return page.evaluate(
    ({ id, which }) => {
      const node = document.querySelector(`[data-audit-scroller="${id}"]`);
      const owner =
        node.dataset.scrollEdges === "frame" ? node.parentElement : node;
      const style = getComputedStyle(owner, which);
      return {
        opacity: Number(style.opacity),
        pointerEvents: style.pointerEvents,
        position: style.position,
        painted: style.backgroundImage !== "none",
        start: node.hasAttribute("data-edge-start"),
        end: node.hasAttribute("data-edge-end"),
      };
    },
    { id, which },
  );
}

async function scrollTo(page, id, where) {
  await page
    .locator(`[data-audit-scroller="${id}"]`)
    .evaluate((node, where) => {
      node.scrollTop = where === "top" ? 0 : node.scrollHeight;
    }, where);
  // The cue updates on the next animation frame; transitions finish in 140ms.
  await page.waitForTimeout(200);
}

/** Every scroller found has the cue, and it appears and hides correctly. */
async function auditScrollers(page, where) {
  const found = await scrollers(page);
  const missing = found.filter((item) => !item.mode).map((item) => item.name);
  expect(missing, `scroll areas without edge cues on ${where}`).toEqual([]);
  for (const { id, name } of found.filter((item) => item.id)) {
    const label = `${where}: ${name}`;
    await scrollTo(page, id, "top");
    const topStart = await lip(page, id, "::before");
    const topEnd = await lip(page, id, "::after");
    expect(topStart.start, `${label} has no top cue at the top`).toBe(false);
    expect(topStart.opacity, `${label} top lip hidden at the top`).toBe(0);
    expect(topEnd.end, `${label} shows more below`).toBe(true);
    expect(topEnd.opacity, `${label} bottom lip visible`).toBe(1);
    for (const edge of [topStart, topEnd]) {
      expect(edge.pointerEvents, `${label} lip ignores input`).toBe("none");
      expect(edge.painted, `${label} lip is painted`).toBe(true);
      expect(["sticky", "absolute"], `${label} lip stays put`).toContain(
        edge.position,
      );
    }
    await scrollTo(page, id, "bottom");
    const endStart = await lip(page, id, "::before");
    const endEnd = await lip(page, id, "::after");
    expect(endStart.start, `${label} shows more above`).toBe(true);
    expect(endStart.opacity, `${label} top lip visible`).toBe(1);
    expect(endEnd.end, `${label} has no bottom cue at the bottom`).toBe(false);
    expect(endEnd.opacity, `${label} bottom lip hidden at the bottom`).toBe(0);
    await scrollTo(page, id, "top");
  }
  return found.filter((item) => item.id).length;
}

for (const size of [
  { width: 1220, height: 860 },
  { width: 740, height: 540 },
]) {
  test(`every page's scroll areas show edge cues at ${size.width}×${size.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(size);
    await openApp(page, {}, "/?theme=fog");
    let total = 0;
    for (const name of pages) {
      await goToPage(page, name);
      total += await auditScrollers(page, name);
    }
    // The page content scrolls on several pages; the sidebar at the minimum.
    expect(total).toBeGreaterThan(size.height < 600 ? pages.length : 2);
  });
}

test("popovers, panels, documents and textareas show edge cues", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1220, height: 700 });
  await openApp(page, {}, "/?page=setup&theme=slate");
  // Model picker list.
  await page.getByRole("button", { name: "Speech model", exact: true }).click();
  const picker = page.getByRole("dialog", { name: "Speech models" });
  await expect(picker).toBeVisible();
  const list = picker.locator(".option-well");
  await expect(list).toHaveAttribute("data-scroll-edges", "inside");
  await auditScrollers(page, "Speech picker");
  await page.keyboard.press("Escape");

  // About: the Terms panel.
  await goToPage(page, "About");
  const terms = await openFold(page, "SonoJot Terms of Use");
  await expect(
    terms.getByRole("region", { name: "Full SonoJot Terms of Use" }),
  ).toHaveAttribute("data-scroll-edges", "inside");
  await auditScrollers(page, "About with Terms");

  // Recent dictations.
  await goToPage(page, "Dictation");
  await showRecentDictations(page);
  await auditScrollers(page, "Recent dictations");

  // Meeting transcript textarea (frame mode): long text makes it scroll.
  await goToPage(page, "Meetings");
  await openMeeting(page, /Design review/);
  const transcript = page.locator("#meeting-transcript");
  if (await transcript.count()) {
    await transcript.fill(
      Array.from(
        { length: 60 },
        (_, line) => `[0:${line}] You: line ${line}`,
      ).join("\n"),
    );
    await expect(transcript).toHaveAttribute("data-scroll-edges", "frame");
  }
  await auditScrollers(page, "Meeting document");
});

test("edge lips never take clicks from the content under them", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1220, height: 700 });
  await openApp(page, {}, "/?page=setup&theme=fog");
  await page.getByRole("button", { name: "Speech model", exact: true }).click();
  const picker = page.getByRole("dialog", { name: "Speech models" });
  const well = picker.locator(".option-well");
  await expect(well).toHaveAttribute("data-edge-end", "");
  // The option partly under the bottom lip still receives the click.
  const box = await well.boundingBox();
  const hit = await page.evaluate(
    ({ x, y }) =>
      document.elementFromPoint(x, y)?.closest('[role="option"]')
        ? true
        : false,
    { x: box.x + box.width / 2, y: box.y + box.height - 6 },
  );
  expect(hit).toBe(true);
});

test("Paper and print off keeps a flat edge cue", async ({ page }) => {
  await openApp(page, {}, "/?page=setup&theme=fog&material=off");
  await page.getByRole("button", { name: "Speech model", exact: true }).click();
  await auditScrollers(page, "Speech picker, Paper and print off");
});

test("Mac model picker sets aside packages this Mac cannot run", async ({
  page,
}) => {
  const mac = macProfileFixture(16);
  const windowsOnly = {
    state: "unavailable",
    installable: false,
    compatibility:
      "This package requires Windows; choose a Metal model on this Mac.",
  };
  await openApp(
    page,
    {
      ...mac,
      settings: { ...mac.settings, setupProfile: "custom" },
      modelPatches: {
        ...mac.modelPatches,
        ...Object.fromEntries(
          [
            "moonshine-medium",
            "mock-whisper-small-cpu",
            "mock-whisper-small-cuda",
            "qwen3-asr-1.7b",
            "qwen3-asr-1.7b-fp16-cuda",
            "whisper-tiny-fp32-cuda",
          ].map((id) => [id, windowsOnly]),
        ),
      },
    },
    "/?page=setup&theme=fog",
  );
  await page.getByRole("button", { name: "Speech model", exact: true }).click();
  const picker = page.getByRole("dialog", { name: "Speech models" });
  const options = picker.getByRole("option");
  await expect(options.filter({ hasText: "Moonshine" })).toHaveCount(0);
  await expect(options.filter({ hasText: "NVIDIA CUDA" })).toHaveCount(0);
  // Needs a download: still an ordinary option.
  await expect(
    options.filter({ hasText: "Qwen3-ASR 1.7B (Q4 Metal)" }),
  ).toContainText("Apple Metal · Not installed");
  // The selected model is a marked slip and is in view.
  const selected = picker.locator('[role="option"][aria-selected="true"]');
  await expect(selected).toContainText("Whisper Small · Metal");
  await expect(selected.locator(".check")).toBeVisible();
  await expect(selected).toBeInViewport();
  await expect(
    picker.getByRole("region", { name: "Details for Whisper Small · Metal" }),
  ).toContainText("Memory:");
  // Set aside in a collapsed group at the end, with the reason.
  const group = picker.locator("details.unavailable");
  await expect(group.locator("summary")).toContainText(
    "Not available on this Mac",
  );
  await expect(group).not.toHaveAttribute("open", "");
  await group.locator("summary").click();
  const list = group.getByRole("list", { name: "Not available on this Mac" });
  await expect(list).toContainText("Moonshine Medium");
  await expect(list).toContainText("This package requires Windows");
});
