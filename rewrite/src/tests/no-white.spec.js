// Design bible, "No flat white": rendered check behind src/lib/no-white.test.ts.
// Every screen in Fog and Slate: no visible flat near-white fill, no light
// stock showing through a translucent well, and highlighted choices stay
// raised paper (textured, with relief), never a flat fill.
import { expect, test } from "@playwright/test";
import { openApp } from "./app-helpers.js";

const routes = [
  { tab: "dictation" },
  { tab: "meetings" },
  { tab: "words" },
  { tab: "settings" },
  ...["input", "speech", "cleanup", "output", "trust"].map((stage) => ({
    tab: "dictation",
    stage,
  })),
  ...["speech", "notes"].map((stage) => ({ tab: "meetings", stage })),
  ...["dictation", "meetings", "look", "memory", "disk", "updates", "help"].map(
    (stage) => ({ tab: "settings", stage }),
  ),
];

/** Visible elements painted with a flat light fill, or a translucent fill
 * over the light stock tile. Runs in the page. */
function flatLightFills() {
  const rgba = (c) => {
    const m = c.match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const [r, g, b, a = 1] = m[1]
      .split(/[\s,/]+/)
      .filter(Boolean)
      .map(Number);
    return { r, g, b, a };
  };
  const out = [];
  for (const el of document.querySelectorAll(".studio *")) {
    const rect = el.getBoundingClientRect();
    if (rect.width < 2 || rect.height < 2) continue;
    const cs = getComputedStyle(el);
    if (cs.visibility === "hidden" || cs.display === "none") continue;
    const c = rgba(cs.backgroundColor);
    if (!c || c.a === 0) continue;
    const textured = cs.backgroundImage.includes("paper");
    const name = `${el.tagName.toLowerCase()}.${String(el.className).split(" ")[0]}`;
    if (!textured && c.a >= 0.5 && Math.min(c.r, c.g, c.b) >= 240)
      out.push(`${name}: flat ${cs.backgroundColor}`);
    if (textured && c.a < 0.5 && Math.max(c.r, c.g, c.b) < 80)
      out.push(`${name}: stock shows through ${cs.backgroundColor}`);
  }
  return out;
}

const raised = (locator) =>
  locator.evaluate((el) => {
    const cs = getComputedStyle(el);
    return {
      textured: cs.backgroundImage.includes("paper"),
      relief: cs.boxShadow !== "none",
    };
  });

for (const theme of ["fog", "slate"]) {
  test(`${theme}: no screen has a flat white fill`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await openApp(page, {}, `/?theme=${theme}`);
    const found = new Set();
    for (const route of routes) {
      await page.evaluate((r) => window.__testNavigatePage(r), route);
      await expect(page.locator(".page-content")).toBeVisible();
      for (const hit of await page.evaluate(flatLightFills))
        found.add(`${JSON.stringify(route)} ${hit}`);
    }
    expect([...found]).toEqual([]);
  });

  test(`${theme}: a highlighted choice is raised paper`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await openApp(page, {}, `/?theme=${theme}`);
    await page.evaluate(() =>
      window.__testNavigatePage({ tab: "dictation", stage: "input" }),
    );
    await page.locator(".picker-well").first().click();
    const option = page.locator(".picker-options button").nth(1);
    await option.hover();
    expect(await raised(option)).toEqual({ textured: true, relief: true });
    await page.keyboard.press("Escape");
    // The Doubtful words shortcut is a pressed well, not a system button.
    await page.evaluate(() =>
      window.__testNavigatePage({ tab: "dictation", stage: "trust" }),
    );
    const well = page.getByRole("button", {
      name: "Change the next doubtful word shortcut",
    });
    expect(
      await well.evaluate((el) => getComputedStyle(el).boxShadow),
    ).toContain("inset");
  });
}
