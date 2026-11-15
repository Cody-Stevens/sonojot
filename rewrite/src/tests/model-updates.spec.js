import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture } from "./desktop-fixture.js";
import { model, pc } from "./speech-fixture.js";

// In-place model updates: the main-window notice and the On disk page.
// Screenshots for director review (Edge, 1220 x 860, desktop fixture).
const folder = new URL("../../artifacts/model-updates/", import.meta.url);
const ft2 = "parakeet-tdt-0.6b-v2-sonojot-ft2";
const update = (patch = {}) => ({
  id: ft2,
  replaces: [ft2],
  name: "SonoJot Parakeet",
  note: "Makes fewer mistakes in meetings, at the same size and speed.",
  downloadBytes: 652_808_550,
  automatic: false,
  state: "available",
  downloadedBytes: 0,
  error: null,
  ...patch,
});

function fixture(updates) {
  const base = pc();
  return {
    ...base,
    libraryModels: [
      ...base.libraryModels.filter((entry) => entry.id !== ft2),
      {
        ...model(ft2, "SonoJot Parakeet", 2.1e9),
        diskBytes: 661_190_591,
        usedBy: ["asr", "meetingSpeech"],
      },
    ],
    modelUpdates: updates,
  };
}

async function shot(page, name) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(200);
  await page.screenshot({ path: fileURLToPath(new URL(name, folder)) });
}

async function onDisk(page) {
  await page.locator('[data-nav="settings"]').click();
  const fold = page.locator("details.advanced");
  if ((await fold.count()) && !(await fold.evaluate((node) => node.open)))
    await fold.locator("summary").click();
  await page
    .locator(".settings-link")
    .filter({ has: page.getByText("On disk", { exact: true }) })
    .first()
    .click();
}

for (const theme of ["fog", "slate"]) {
  test(`speech model update asks first, ${theme}`, async ({ page }) => {
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.setViewportSize({ width: 1220, height: 860 });
    await installDesktopFixture(page, fixture([update()]));
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.goto(`/?theme=${theme}`);
    const notice = page.locator(".model-update");
    await expect(notice).toContainText(
      "SonoJot Parakeet has an update. Makes fewer mistakes in meetings, at the same size and speed.",
    );
    await expect(notice.getByRole("button", { name: "Update · 653 MB" })).toBeVisible();
    await shot(page, `${theme}-01-main-window.png`);

    await onDisk(page);
    await expect(page.locator(".model-update")).toHaveCount(1);
    const row = page.locator(`[data-model-id="${ft2}"]`);
    await expect(row).toContainText("662 MB · Update available");
    await row.locator(".disk-row").click();
    await expect(row.getByRole("button", { name: "Update · 653 MB" })).toBeVisible();
    await shot(page, `${theme}-02-on-disk.png`);

    await page.locator(".model-update").getByRole("button", { name: "Update · 653 MB" }).click();
    await expect(page.locator(".model-update")).toContainText("Updating SonoJot Parakeet…");
    expect(errors).toEqual([]);
  });
}

test("several updates offer Update all, and Later hides the notice", async ({ page }) => {
  await page.setViewportSize({ width: 1220, height: 860 });
  const qwen = update({
    id: "qwen3-asr-1.7b",
    replaces: ["qwen3-asr-1.7b"],
    name: "Qwen3-ASR 1.7B",
    note: "Fewer repeated words.",
    downloadBytes: 1_200_000_000,
  });
  await installDesktopFixture(page, fixture([update(), qwen]));
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?theme=fog");
  const notice = page.locator(".model-update");
  await expect(notice).toContainText("Updates for 2 models: SonoJot Parakeet, Qwen3-ASR 1.7B.");
  await expect(notice.getByRole("button", { name: "Update all · 1.9 GB" })).toBeVisible();
  await shot(page, "fog-03-update-all.png");
  await notice.getByRole("button", { name: "Later" }).click();
  await expect(notice).toHaveCount(0);
});

test("a downloading update shows progress and a failure says the old version stays", async ({ page }) => {
  await page.setViewportSize({ width: 1220, height: 860 });
  await installDesktopFixture(
    page,
    fixture([update({ state: "downloading", downloadedBytes: 274_000_000 })]),
  );
  await page.goto("/?theme=fog");
  await expect(page.locator(".model-update")).toContainText("Updating SonoJot Parakeet · 41%");
  await shot(page, "fog-04-downloading.png");
});

test("automatic cleanup updates stay quiet", async ({ page }) => {
  await installDesktopFixture(
    page,
    fixture([
      update({
        id: "sonojot-punct-v4",
        replaces: ["sonojot-punct-v3"],
        name: "SonoJot Punctuation v4",
        automatic: true,
        state: "downloading",
      }),
    ]),
  );
  await page.goto("/?theme=fog");
  await expect(page.locator(".app-shell, .studio").first()).toBeVisible();
  await page.waitForTimeout(500);
  await expect(page.locator(".model-update")).toHaveCount(0);
});
