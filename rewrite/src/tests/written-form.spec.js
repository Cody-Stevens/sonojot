import { expect, test } from "@playwright/test";
import { openApp, openMoreOptions } from "./app-helpers.js";

test("Written form saves its choice and applies only to Clean", async ({
  page,
}) => {
  await openApp(page, {}, "/?page=dictation");
  await openMoreOptions(page);
  const toggle = page.getByRole("switch", {
    name: "Written form",
    exact: true,
  });
  await expect(toggle).toBeEnabled();
  await expect(toggle).toBeChecked();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  const modes = page.getByRole("group", { name: "Dictation cleanup mode" });
  for (const name of [/^Format only/, /^As heard/]) {
    await modes.getByRole("button", { name }).click();
    await expect(toggle).toBeDisabled();
    await expect(toggle).not.toBeChecked();
  }
  await modes.getByRole("button", { name: /^Clean/ }).click();
  await expect(toggle).toBeEnabled();
  await toggle.click();
  await expect(toggle).toBeChecked();
  expect(
    await page.evaluate(
      () =>
        window.__testCalls
          .filter((call) => call.command === "save_settings")
          .at(-1).args.settings.writtenForm,
    ),
  ).toBe(true);
  await expect(toggle).toBeEnabled();
  await page.screenshot({
    path: "../artifacts/written-form/dictation-fog.png",
    fullPage: true,
  });
  await page.evaluate(() => window.__testPushSnapshot({
    settings: { ...window.__testSnapshot().settings, theme: "slate" },
  }));
  await expect(page.locator(".studio")).toHaveAttribute("data-theme", "slate");
  await page.screenshot({
    path: "../artifacts/written-form/dictation-slate.png",
    fullPage: true,
  });
});
