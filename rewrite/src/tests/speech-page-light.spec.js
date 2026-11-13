import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";
import { speechProfiles, ft2 } from "./speech-page-fixture.js";

// November 13 light Speech page: information buttons and Compare models.
async function open(page, hw = "nvidia", size = { width: 1220, height: 860 }) {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize(size);
  await installDesktopFixture(page, speechProfiles(false)[hw]);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?page=setup&theme=fog");
  await expect(
    page.getByRole("heading", { name: "Speech", exact: true }),
  ).toBeVisible();
  return errors;
}

test("the Speech page has no chart and one short line per card", async ({
  page,
}) => {
  const errors = await open(page);
  await expect(page.locator(".speech-screen svg")).toHaveCount(0);
  for (const line of await page
    .locator(".card-choose .model-copy > small")
    .allTextContents())
    expect(line.trim().split(/\s+/).length).toBeLessThanOrEqual(8);
  expect(errors).toEqual([]);
});

test("information buttons open on click or Enter and Space, not on focus, and close on Escape or outside", async ({
  page,
}) => {
  await open(page);
  const button = page.getByRole("button", { name: "About SonoJot Parakeet" });
  const note = page.getByRole("note", { name: "About SonoJot Parakeet" });
  await expect(note).toBeHidden();
  await button.click();
  await expect(note).toBeVisible();
  await expect(button).toHaveAttribute("aria-expanded", "true");
  // One line per kind of speech, each naming its test, plus meetings.
  await expect(note).toContainText(
    "7.54 per 100 words (People's Speech, Common Voice)",
  );
  await expect(note).toContainText("16.28 per 100 words (VoxPopuli)");
  await expect(note).toContainText("3.89 per 100 words (12 TED talks)");
  await expect(note).toContainText(
    "10.14 per 100 words (AMI, NOTSOFAR, DiPCo, pooled)",
  );
  await expect(note).toContainText("ParakeetMeetingsFp16.md");
  // The note never chooses the card under it.
  await expect(
    page.locator(`[data-model-id="${ft2}"] .card-choose`),
  ).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("Escape");
  await expect(note).toBeHidden();
  await expect(button).toBeFocused();
  await button.click();
  await page.mouse.click(5, 400);
  await expect(note).toBeHidden();
  // Keyboard focus alone does not open it; Enter and Space do.
  await page.locator(`[data-model-id="${ft2}"] .card-choose`).focus();
  await page.keyboard.press("Tab");
  const focused = await page.evaluate(() =>
    document.activeElement?.getAttribute("aria-label"),
  );
  expect(focused).toBe("About SonoJot Parakeet");
  await expect(note).toBeHidden();
  await page.keyboard.press("Enter");
  await expect(note).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(note).toBeHidden();
  await page.keyboard.press("Space");
  await expect(note).toBeVisible();
  // Moving on closes it.
  await page.keyboard.press("Tab");
  await expect(note).toBeHidden();
});

test("Compare models plots measured models once, marks the frontier and offers a table", async ({
  page,
}) => {
  await open(page);
  await page.getByRole("button", { name: "Compare models" }).click();
  const sheet = page.getByRole("dialog", { name: "Compare models" });
  const marks = sheet.locator("[data-chart-model]");
  // Everyday speech first: the five packages measured in that one run.
  const kind = sheet.getByRole("group", { name: "Kind of speech" });
  await expect(
    kind.getByRole("button", { name: "Everyday speech" }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(marks).toHaveCount(5);
  await expect(sheet.locator(".frontier")).toHaveCount(1);
  await expect(sheet).toContainText(
    "Everyday speech is shown first because it is the public test closest to dictation.",
  );
  const labels = (await sheet.locator(".label").allTextContents()).join(" ");
  for (const name of ["Qwen 1.7B Q8", "SonoJot Parakeet", "Qwen 1.7B Q4"])
    expect(labels).toContain(name);
  await kind.getByRole("button", { name: "Accented English" }).click();
  await expect(sheet).toContainText("trained on other VoxPopuli speakers");
  await kind.getByRole("button", { name: "Talks" }).click();
  await expect(marks).toHaveCount(5);
  await expect(sheet.locator(`[data-chart-model="moonshine-small"]`)).toHaveCount(0);
  // Every mark is a keyboard stop with the exact numbers.
  await marks.first().focus();
  await expect(sheet.locator(".readout")).toContainText("word errors per 100");
  await sheet.getByRole("button", { name: "Table", exact: true }).click();
  const table = sheet.getByRole("table");
  await expect(table).toBeVisible();
  await expect(
    table.locator('[data-table-model="whisper-large-v3-turbo"]'),
  ).toContainText("Not measured");
  await expect(table.locator(`[data-table-model="${ft2}"]`)).toContainText(
    "3.89%",
  );
  await expect(
    table.locator('[data-table-model="qwen3-asr-1.7b-q8-cuda"]'),
  ).toContainText("2.6%");
  await expect(
    table.locator('[data-table-model="moonshine-small"]'),
  ).toContainText("8.4% (another run)");
});

test("on a Mac only Mac-app measurements are plotted, and it fits the minimum window", async ({
  page,
}) => {
  await open(page, "mac", { width: 740, height: 540 });
  await page.getByRole("button", { name: "Compare models" }).click();
  const sheet = page.getByRole("dialog", { name: "Compare models" });
  const ids = await sheet
    .locator("[data-chart-model]")
    .evaluateAll((nodes) => nodes.map((n) => n.dataset.chartModel));
  // The November 13 M4 run: SonoJot Parakeet, Qwen Q4 and Q8, all measured.
  expect(ids.sort()).toEqual(
    [ft2, "qwen3-asr-1.7b-q4-metal", "qwen3-asr-1.7b-q8-metal"].sort(),
  );
  await expect(sheet.getByRole("group", { name: "Kind of speech" })).toHaveCount(0);
  await expect(sheet).toContainText("Only talks are measured in the Mac app so far.");
  const box = await sheet.locator(".pareto svg").boundingBox();
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(740);
});
