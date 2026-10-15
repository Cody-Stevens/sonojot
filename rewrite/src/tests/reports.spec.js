import { test, expect } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";

const pendingCrash = {
  id: "602fe893-2af7-4d16-94f0-e43621f95971", reason: "panic",
  app: { version: "0.28.6", platform: "linux", arch: "x86_64", os: "Arch Linux" },
  crash: { summary: "index out of bounds", location: "speech.rs:42", backtrace: "0: sonojot::speech" },
};
const calls = (page, command) => page.evaluate((command) => window.__testCalls.filter(c => c.command === command), command);

test("crash launch requires consent, previews the payload and safely retries", async ({ page }) => {
  await installDesktopFixture(page, { pendingCrash, capabilities: { platform: "linux" }, reportFailures: 1 });
  await page.goto("/?page=report");
  await expect(page.getByRole("heading", { name: "SonoJot crashed" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Feedback Alpha" })).toBeVisible();
  const send = page.getByRole("button", { name: "Send", exact: true });
  await expect(send).toBeDisabled();
  await page.screenshot({ path: "../artifacts/reports-crash-fog.png" });
  expect(await calls(page, "send_report")).toHaveLength(0);
  await page.getByRole("checkbox", { name: "Send this anonymous crash report to help fix bugs" }).check();
  await page.getByText("Show what will be sent", { exact: true }).click();
  await expect(send).toBeEnabled();
  const shown = JSON.parse(await page.locator(".report-preview pre").innerText());
  await send.click();
  await expect(page.getByRole("button", { name: "Copy report" })).toBeVisible();
  await page.getByRole("button", { name: "Copy report" }).click();
  expect((await calls(page, "copy_report"))[0].args.report).toEqual(shown);
  await send.click();
  await expect(page.getByText("Report sent. Receipt:")).toBeVisible();
  const sent = await calls(page, "send_report");
  expect(sent).toHaveLength(2);
  expect(sent[0].args.report).toEqual(shown);
  expect(sent[1].args.report).toEqual(shown);
  expect((await calls(page, "dismiss_crash"))[0].args.id).toBe(pendingCrash.id);
});

test("bug entry on Windows excludes diagnostics until opted in and edits get a new ID", async ({ page }) => {
  await installDesktopFixture(page, { capabilities: { platform: "windows" }, reportFailures: 2 });
  await page.goto("/?page=about&theme=slate");
  await expect(page.getByRole("button", { name: "Feedback Alpha" })).toHaveCount(0);
  await page.getByRole("button", { name: "Report a bug" }).click();
  const send = page.getByRole("button", { name: "Send", exact: true });
  await expect(send).toBeDisabled();
  await page.getByRole("textbox", { name: "What went wrong?" }).fill("The shortcut stopped working.");
  await send.click();
  await expect(page.getByRole("button", { name: "Copy report" })).toBeVisible();
  await page.screenshot({ path: "../artifacts/reports-bug-slate.png" });
  await page.getByRole("textbox", { name: "What went wrong?" }).fill("The shortcut stopped after waking.");
  await send.click();
  const sent = await calls(page, "send_report");
  expect(sent[0].args.report.diagnostics).toBeNull();
  expect(sent[1].args.report.requestId).not.toBe(sent[0].args.report.requestId);
});

test("stale marker dismisses without sending; onboarding takes priority", async ({ page }) => {
  await installDesktopFixture(page, { pendingCrash: { ...pendingCrash, reason: "unexpectedExit" } });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "SonoJot closed unexpectedly" })).toBeVisible();
  await page.getByRole("button", { name: "Not now" }).click();
  await expect(page.getByRole("heading", { name: "About", exact: true })).toBeVisible();
  expect(await calls(page, "send_report")).toHaveLength(0);
  expect(await calls(page, "dismiss_crash")).toHaveLength(1);
});

test("crash prompt never replaces first-run setup", async ({ page }) => {
  await installDesktopFixture(page, { pendingCrash, firstRun: true });
  await page.goto("/?page=report");
  await expect.poll(() => calls(page, "get_pending_crash")).toHaveLength(1);
  await expect(page.getByRole("heading", { name: "SonoJot crashed" })).toHaveCount(0);
  expect(await calls(page, "prepare_report")).toHaveLength(0);
  expect(await calls(page, "send_report")).toHaveLength(0);
});
