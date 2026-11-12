import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";

test("a shaped result keeps session-checked copy-only recovery", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    settings: { smartLayout: true, doNotSaveDictations: true, autoPaste: true },
    history: [],
  });
  await page.goto("/?view=overlay&theme=fog");
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "idle",
      resultSessionId: 42,
      sessionId: null,
      recognitionCompleteness: "complete",
      publicationStage: "smart_layout",
      originalText: "hi alex please send the draft thanks sam",
      preModelText: "Hi Alex please send the draft thanks Sam.",
      outputText: "Hi Alex,\n\nPlease send the draft.\n\nThanks,\nSam",
      guardReason: null,
      tabVisible: true,
      tabExpanded: true,
      tabTemporary: true,
      canCopyLast: true,
    }),
  );
  const recovery = page.getByRole("button", {
    name: "Copy before AI changes",
    exact: true,
  });
  await expect(recovery).toBeVisible();
  await recovery.click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__testCalls
          .filter((call) => call.command === "copy_smart_original")
          .map((call) => call.args),
      ),
    )
    .toEqual([{ resultSessionId: 42 }]);
  await expect(page.getByText("Copied", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__testClipboard)).toBe(
    "Hi Alex please send the draft thanks Sam.",
  );
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter((call) => call.command === "copy_text"),
    ),
  ).toHaveLength(0);
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "listening",
      sessionId: 43,
      resultSessionId: null,
    }),
  );
  await expect(recovery).toHaveCount(0);
});
