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

test.beforeEach(async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
});

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
    const modal = document.querySelector("dialog[open]");
    let index = 0;
    for (const element of document.querySelectorAll("body *")) {
      if (modal && !modal.contains(element)) continue;
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
  // Wait for the scroll observer, including on a busy renderer.
  await expect
    .poll(async () => {
      const edge = await lip(page, id, "::before");
      return { start: edge.start, end: edge.end };
    })
    .toEqual({ start: where === "bottom", end: where === "top" });
}

/** Every scroller found has the cue, and it appears and hides correctly. */
async function auditScrollers(page, where) {
  await page.evaluate(() => document.fonts.ready);
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
    expect(topEnd.opacity, `${label} bottom lip visible`).toBeGreaterThan(0);
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
    expect(endStart.opacity, `${label} top lip visible`).toBeGreaterThan(0);
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
    // At the minimum size most pages scroll. Since the November 13 light
    // Speech page, no main page overflows at 1220×860; the sheets and long
    // documents below keep that size covered.
    if (size.height < 600) expect(total).toBeGreaterThan(pages.length - 2);
  });
}

test("popovers, panels, documents and textareas show edge cues", async ({
  page,
}) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 1220, height: 700 });
  await openApp(
    page,
    { meeting: { notes: "# Summary\nA saved note." } },
    "/?page=setup&theme=slate",
  );
  // The Compare models sheet (chart and table).
  await page
    .getByRole("button", { name: "Compare models", exact: true })
    .click();
  const picker = page.getByRole("dialog", { name: "Compare models" });
  await expect(picker).toBeVisible();
  const list = picker.locator(".sheet-scroll");
  await expect(list).toHaveAttribute("data-scroll-edges", "inside");
  await auditScrollers(page, "Compare models sheet");
  await page.keyboard.press("Escape");

  // Help: the Terms panel.
  await goToPage(page, "About");
  await openFold(page, "Troubleshooting and legal");
  const terms = await openFold(page, "Terms of Use");
  await expect(
    terms.getByRole("region", { name: "Full SonoJot Terms of Use" }),
  ).toHaveAttribute("data-scroll-edges", "inside");
  await auditScrollers(page, "Help with Terms");

  // Recent dictations.
  await goToPage(page, "Dictation");
  await showRecentDictations(page);
  await auditScrollers(page, "Recent dictations");

  // The transcript and notes editors keep their frame cues with long text.
  await goToPage(page, "Meetings");
  await openMeeting(page, /Design review/);
  await page.getByRole("tab", { name: "Transcript", exact: true }).click();
  const transcript = page.getByRole("textbox", {
    name: "Meeting transcript",
    exact: true,
  });
  const original = await transcript.inputValue();
  await transcript.fill(
    Array.from(
      { length: 60 },
      (_, line) => `[0:${line}] You: line ${line}`,
    ).join("\n"),
  );
  await expect(transcript).toHaveAttribute("data-scroll-edges", "frame");
  await auditScrollers(page, "Meeting document");
  await transcript.fill(original);
  await page.getByRole("tab", { name: "Notes", exact: true }).click();
  await page.getByRole("button", { name: "Edit notes", exact: true }).click();
  const notes = page.getByRole("textbox", {
    name: "Meeting notes",
    exact: true,
  });
  await notes.fill(
    Array.from({ length: 60 }, (_, line) => `Note ${line}`).join("\n"),
  );
  await expect(notes).toHaveAttribute("data-scroll-edges", "frame");
  await auditScrollers(page, "Meeting notes editor");
});

test("edge lips never take clicks from the content under them", async ({
  page,
}) => {
  // The minimum window height, where the Compare models sheet scrolls.
  await page.setViewportSize({ width: 740, height: 540 });
  await openApp(page, {}, "/?page=setup&theme=fog");
  await page
    .getByRole("button", { name: "Compare models", exact: true })
    .click();
  const picker = page.getByRole("dialog", { name: "Compare models" });
  await picker.getByRole("button", { name: "Table", exact: true }).click();
  const well = picker.locator(".sheet-scroll");
  await expect(well).toHaveAttribute("data-edge-end", "");
  const edges = await well.evaluate((element) => ({
    top: getComputedStyle(element, "::before").pointerEvents,
    bottom: getComputedStyle(element, "::after").pointerEvents,
  }));
  expect(edges).toEqual({ top: "none", bottom: "none" });
  await picker.getByRole("button", { name: "Close sheet" }).click();
  await expect(picker).toHaveCount(0);
});

test("Paper and print off keeps a flat edge cue", async ({ page }) => {
  await openApp(page, {}, "/?page=setup&theme=fog&material=off");
  await page
    .getByRole("button", { name: "Compare models", exact: true })
    .click();
  await auditScrollers(page, "Compare models sheet, Paper and print off");
});
