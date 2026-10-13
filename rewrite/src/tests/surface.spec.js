import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";

test("paper header controls call the window API without starting a drag", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/");
  await page
    .getByRole("button", { name: "Minimize window", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Maximize window", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Restore window", exact: true }),
  ).toBeVisible();
  const beforeDrag = await page.evaluate(() => window.__testCalls);
  expect(
    beforeDrag.some((call) => call.command === "plugin:window|start_dragging"),
  ).toBe(false);
  await page.locator(".brand").dblclick();
  await expect(
    page.getByRole("button", { name: "Maximize window", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Close window", exact: true }).click();
  const calls = await page.evaluate(() =>
    window.__testCalls.map((call) => call.command),
  );
  expect(calls).toContain("plugin:window|minimize");
  expect(calls).toContain("plugin:window|close");
  expect(calls).toContain("plugin:window|start_dragging");
  await expect(page.locator(".sprig,.nav-end")).toHaveCount(0);
  const surface = await page.evaluate(() => ({
    padding: getComputedStyle(document.querySelector(".studio")).padding,
    background: getComputedStyle(document.documentElement).backgroundColor,
    left: document.querySelector(".sheet").getBoundingClientRect().left,
  }));
  expect(surface).toEqual({
    padding: "0px",
    background: "rgba(0, 0, 0, 0)",
    left: 0,
  });
});

test("screen tab shows a bounded literal newest suffix without changing recognition", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.setViewportSize({ width: 360, height: 272 });
  await page.goto("/?view=overlay");
  const original =
    "Earlier words … " +
    "keep these words in their original order ".repeat(12) +
    "newest words 🪶";
  await page.evaluate(
    (text) =>
      window.__testPushSnapshot({
        state: "listening",
        interimText: text,
        tabVisible: true,
        tabExpanded: true,
      }),
    original,
  );
  const preview = page.locator(".tab-preview");
  await expect(preview.locator("span")).toHaveText(
    Array.from(original).slice(-280).join(""),
  );
  const edge = await preview.evaluate((element) => ({
    bottom: element.firstElementChild.getBoundingClientRect().bottom,
    expected: element.getBoundingClientRect().bottom,
    direction: getComputedStyle(element).direction,
  }));
  expect(Math.abs(edge.bottom - edge.expected)).toBeLessThan(1);
  expect(edge.direction).toBe("ltr");
  expect(await page.evaluate(() => window.__testSnapshot().interimText)).toBe(
    original,
  );
  await page.evaluate(() =>
    window.__testPushSnapshot({ interimText: "A short line." }),
  );
  await expect(preview.locator("span")).toHaveText("A short line.");
});

test("all meters use decibel normalization and stay dark at silence", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?page=microphone");
  await page.evaluate(() =>
    window.__testPushSnapshot({ state: "listening", audioLevel: 0.01 }),
  );
  const meter = page.getByRole("meter");
  await expect(meter).toHaveAttribute("aria-valuenow", "42");
  await expect(meter).toHaveAttribute("aria-valuetext", "-40 dBFS");
  expect(await page.evaluate(() => window.__testSnapshot().audioLevel)).toBe(
    0.01,
  );
  await page.evaluate(() => window.__testPushSnapshot({ audioLevel: 0 }));
  await expect(meter).toHaveAttribute("aria-valuenow", "0");
  await expect(meter.locator(".lit")).toHaveCount(0);
});
