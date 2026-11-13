import { test, expect } from "@playwright/test";
import { join } from "node:path";
import { mkdirSync } from "node:fs";
const screenshotDir = process.env.SONOJOT_SYNC_SCREENSHOTS;
async function screenshot(page, name) {
  if (!screenshotDir) return;
  mkdirSync(screenshotDir, { recursive: true });
  await page.screenshot({
    path: join(screenshotDir, name + ".png"),
    fullPage: true,
  });
}
const locked = {
  signedIn: true,
  unlocked: false,
  initialized: true,
  email: "fixture@example.invalid",
  device: "b",
  server: "https://sync.example.invalid",
};
const trusted = {
  ...locked,
  unlocked: true,
  device: "a",
  lastSync: 1794000000,
};
const request = () => ({
  id: "fixture-pair",
  device: "b",
  platform: "Windows",
  name: "Study computer",
  state: "committed",
  expires_at: Math.floor(Date.now() / 1000) + 300,
});
const operations = (page, operation) =>
  page.evaluate(
    (operation) =>
      window.__testCalls
        .filter(
          (c) => c.command === "sync_command" && c.args.operation === operation,
        )
        .map((c) => c.args.args),
    operation,
  );
import { installDesktopFixture } from "./desktop-fixture.js";
for (const pageName of ["settings", "sync", "vocabulary"]) {
  test(`stable builds hide sync on ${pageName} and never invoke it`, async ({
    page,
  }) => {
    await installDesktopFixture(page, {
      syncEnabled: false,
      syncStatus: {
        signedIn: true,
        unlocked: true,
        server: "https://sync.example.invalid",
      },
    });
    await page.goto(`/?page=${pageName}&flavor=synctest`);
    await expect(
      page.getByRole("heading", { name: "Account and sync" }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Account and sync", exact: false }),
    ).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Sync now" })).toHaveCount(0);
    expect(
      await page.evaluate(() =>
        window.__testCalls.filter((c) => c.command === "sync_command"),
      ),
    ).toHaveLength(0);
  });
}
test("Sync Test opens Account and sync with a visible flavor label", async ({
  page,
}) => {
  await installDesktopFixture(page, { syncEnabled: true });
  await page.goto("/?page=sync&flavor=synctest");
  await expect(
    page.getByRole("heading", { name: "Account and sync" }),
  ).toBeVisible();
  await expect(page.locator(".brand")).toContainText("Sync Test");
  await expect(page.getByRole("button", { name: "Set up sync" })).toBeVisible();
  await page.screenshot({
    path: "artifacts/sync-test-flavor.png",
    fullPage: true,
  });
});
test("account screen separates setup and adding a computer", async ({
  page,
}) => {
  await installDesktopFixture(page, { syncEnabled: true });
  await page.goto("/?page=settings");
  await page
    .getByRole("button", { name: "Account and sync", exact: false })
    .click();
  await expect(
    page.getByRole("heading", { name: "Account and sync" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Set up sync" }).click();
  expect(
    await page.evaluate(() =>
      window.__testCalls
        .filter(
          (c) => c.command === "sync_command" && c.args.operation === "signin",
        )
        .map((c) => c.args.args.intent),
    ),
  ).toEqual(["sign-up"]);
  await page.getByRole("button", { name: "Add this computer" }).click();
  expect(
    await page.evaluate(() =>
      window.__testCalls
        .filter(
          (c) => c.command === "sync_command" && c.args.operation === "signin",
        )
        .map((c) => c.args.args.intent),
    ),
  ).toEqual(["sign-up", "sign-in"]);
  await expect(page.getByRole("alert")).toContainText("not configured");
  await expect(page.getByRole("button", { name: "Try again" })).toBeEnabled();
  await page.screenshot({
    path: "artifacts/sync-signed-out.png",
    fullPage: true,
  });
});
test("removing a device needs a confirmation and can re-key", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: {
      signedIn: true,
      unlocked: true,
      email: "fixture@example.invalid",
      device: "other",
      server: "https://sync.example.invalid",
      lastSync: 1794000000,
    },
  });
  await page.goto("/?page=settings");
  await page
    .getByRole("button", { name: "Account and sync", exact: false })
    .click();
  await page
    .getByRole("button", { name: "Account options", exact: true })
    .click();
  await page.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByLabel("Sign out only, no re-key")).not.toBeChecked();
  await page.screenshot({ path: "artifacts/sync-rekey.png", fullPage: true });
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter(
        (c) => c.command === "sync_command" && c.args.operation === "remove",
      ),
    ),
  ).toHaveLength(0);
});

test("second computer starts automatically after sign-in and shows digits, waiting and Cancel", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncSigninStatus: locked,
  });
  await page.goto("/?page=sync&flavor=synctest&theme=fog");
  await page.getByRole("button", { name: "Add this computer" }).click();
  await expect(
    page.getByRole("heading", { name: "Approve on another device" }),
  ).toBeVisible();
  await expect(page.locator(".pair-code")).toHaveText("654 321");
  await expect(
    page.getByRole("status").filter({ hasText: "Waiting for approval…" }),
  ).toBeVisible();
  await expect(page.locator(".spinner")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeEnabled();
  await expect(
    page.getByText(
      "On a computer where SonoJot sync is already set up, open Settings → Account and sync and approve this computer. Check that it shows the same six digits.",
    ),
  ).toBeVisible();
  for (const name of [
    "Compare six digits",
    "Check pairing",
    "They match",
    "Approve",
    "Account options",
    "Download account data",
    "Delete account",
  ])
    await expect(page.getByRole("button", { name, exact: true })).toHaveCount(
      0,
    );
  await expect(page.locator(".sync-sheet")).toHaveCount(1);
  expect(await operations(page, "pair_start")).toHaveLength(1);
  expect(await operations(page, "pair_approve")).toHaveLength(0);
  expect(await operations(page, "pair_confirm")).toHaveLength(0);
  await screenshot(page, "new-computer");
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByText("Approval cancelled.")).toBeVisible();
  await expect(page.locator(".pair-code")).toHaveCount(0);
  expect(await operations(page, "pair_deny")).toEqual([{ id: "fixture-pair" }]);
});

test("sign-in callback starts approval without reloading Settings", async ({
  page,
}) => {
  await installDesktopFixture(page, { syncEnabled: true });
  await page.goto("/?page=sync&flavor=synctest");
  await expect(
    page.getByRole("button", { name: "Add this computer" }),
  ).toBeVisible();
  await page.evaluate((status) => window.__testSyncChange({ status }), locked);
  await expect(page.locator(".pair-code")).toHaveText("654 321");
});

for (const decision of ["Approve", "Deny"])
  test(`trusted computer shows digits and ${decision} directly`, async ({
    page,
  }) => {
    await installDesktopFixture(page, {
      syncEnabled: true,
      syncStatus: trusted,
      syncPairs: [request()],
    });
    await page.goto("/?page=sync&flavor=synctest&theme=fog");
    await expect(
      page.getByRole("heading", { name: "A new computer wants to join sync" }),
    ).toBeVisible();
    await expect(page.getByText("Study computer · Windows")).toBeVisible();
    await expect(page.locator(".pair-code")).toHaveText("654 321");
    await expect(
      page.getByRole("button", { name: "Approve", exact: true }),
    ).toBeEnabled();
    await expect(
      page.getByRole("button", { name: "Deny", exact: true }),
    ).toBeEnabled();
    expect(await operations(page, "pair_approve")).toHaveLength(0);
    await screenshot(page, "trusted-computer");
    await page.getByRole("button", { name: decision, exact: true }).click();
    expect(
      await operations(
        page,
        decision === "Approve" ? "pair_approve" : "pair_deny",
      ),
    ).toEqual([
      {
        id: "fixture-pair",
        ...(decision === "Approve" ? { confirmed: true } : {}),
      },
    ]);
    await expect(page.locator(".pair-code")).toHaveCount(0);
  });

test("trusted computer discovers requests while Settings stays open", async ({
  page,
}) => {
  await installDesktopFixture(page, { syncEnabled: true, syncStatus: trusted });
  await page.goto("/?page=sync&flavor=synctest");
  await expect(
    page.getByRole("button", { name: "Account options" }),
  ).toBeVisible();
  await page.evaluate(
    (pairs) => window.__testSyncChange({ pairs }),
    [request()],
  );
  await expect(page.locator(".pair-code")).toHaveText("654 321");
});

test("commitment exchange runs automatically before digits are shown", async ({
  page,
}) => {
  await page.clock.install();
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: locked,
    syncPairPolls: [{ waiting: true }, { waiting: true }, { code: "012345" }],
  });
  await page.goto("/?page=sync&flavor=synctest");
  await expect(page.getByText("Preparing six digits…")).toBeVisible();
  await expect
    .poll(async () => (await operations(page, "pair_poll")).length)
    .toBe(1);
  await expect(page.locator(".pair-code")).toHaveCount(0);
  await page.clock.fastForward(3100);
  await expect
    .poll(async () => (await operations(page, "pair_poll")).length)
    .toBe(2);
  await page.clock.fastForward(3100);
  await expect(page.locator(".pair-code")).toHaveText("012 345");
});

test("receiver unlocks by polling only after trusted approval", async ({
  page,
}) => {
  await page.clock.install();
  await installDesktopFixture(page, { syncEnabled: true, syncStatus: locked });
  await page.goto("/?page=sync&flavor=synctest");
  await expect(page.locator(".pair-code")).toHaveText("654 321");
  await page.evaluate(() =>
    window.__testSyncChange({ polls: [{ unlocked: true }] }),
  );
  await page.clock.fastForward(3100);
  await expect(
    page.getByRole("button", { name: "Sync now", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".pair-code")).toHaveCount(0);
  expect(await operations(page, "pair_confirm")).toHaveLength(0);
  expect(await operations(page, "pair_approve")).toHaveLength(0);
});

test("expired codes clear and offer Get a new code", async ({ page }) => {
  await page.clock.install();
  await installDesktopFixture(page, { syncEnabled: true, syncStatus: locked });
  await page.goto("/?page=sync&flavor=synctest");
  await expect(page.locator(".pair-code")).toHaveText("654 321");
  await page.clock.fastForward(301000);
  await expect(page.getByText("This code expired.")).toBeVisible();
  await expect(page.locator(".pair-code")).toHaveCount(0);
  await page.getByRole("button", { name: "Get a new code" }).click();
  await expect(page.locator(".pair-code")).toHaveText("654 321");
  expect(await operations(page, "pair_start")).toHaveLength(2);
});

test("pairing errors can retry without starting another commitment", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: locked,
    syncPairError: "Connection interrupted. Try again.",
  });
  await page.goto("/?page=sync&flavor=synctest");
  await expect(page.getByRole("alert")).toContainText("Connection interrupted");
  await page.getByRole("button", { name: "Try again", exact: true }).click();
  await expect(page.locator(".pair-code")).toHaveText("654 321");
  expect(await operations(page, "pair_start")).toHaveLength(1);
});

test("recovery and reset alternatives stay hidden until requested", async ({
  page,
}) => {
  await installDesktopFixture(page, { syncEnabled: true, syncStatus: locked });
  await page.goto("/?page=sync&flavor=synctest&theme=fog");
  await expect(
    page.getByRole("button", { name: "Sign out", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Use recovery code", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "I lost my devices and recovery code" }),
  ).toHaveCount(0);
  await expect(page.getByLabel("Recovery code", { exact: true })).toHaveCount(
    0,
  );
  await page
    .getByRole("button", { name: "Can't approve from another computer?" })
    .click();
  await expect(
    page.getByRole("button", { name: "Use recovery code", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "I lost my devices and recovery code" }),
  ).toBeVisible();
  await screenshot(page, "approval-alternatives");
  await page
    .getByRole("button", { name: "Use recovery code", exact: true })
    .click();
  await page
    .getByLabel("Recovery code", { exact: true })
    .fill("SYNTHETIC-RECOVERY-FIXTURE");
  await page.getByRole("button", { name: "Unlock", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Sync now", exact: true }),
  ).toBeVisible();
  expect(await operations(page, "recovery_use")).toHaveLength(1);
});

test("unlocked account management stays behind Account options", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: { ...trusted, recoveryPending: true },
  });
  await page.goto("/?page=sync&flavor=synctest&theme=fog");
  const options = page.getByRole("button", {
    name: "Account options",
    exact: true,
  });
  await expect(options).toHaveAttribute("aria-expanded", "false");
  for (const name of [
    "New recovery code",
    "Save recovery code",
    "Remove",
    "Sign out",
    "Download account data",
    "Delete account",
  ])
    await expect(page.getByRole("button", { name, exact: true })).toHaveCount(
      0,
    );
  expect(await operations(page, "recovery_show")).toHaveLength(0);
  await screenshot(page, "unlocked-account");
  await options.click();
  for (const name of [
    "New recovery code",
    "Save recovery code",
    "Remove",
    "Sign out",
    "Download account data",
    "Delete account",
  ])
    await expect(page.getByRole("button", { name, exact: true })).toBeVisible();
  await page
    .getByRole("button", { name: "Delete account", exact: true })
    .scrollIntoViewIfNeeded();
  await screenshot(page, "account-options");
  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Add this computer" }),
  ).toBeVisible();
  await expect(options).toHaveCount(0);
});

test("existing device sees and cancels a pending reset", async ({ page }) => {
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: {
      signedIn: true,
      unlocked: true,
      email: "fixture@example.invalid",
      device: "a",
      server: "https://sync.example.invalid",
      pendingReset: 1794260000,
    },
  });
  await page.goto("/?page=settings");
  await page
    .getByRole("button", { name: "Account and sync", exact: false })
    .click();
  await expect(page.getByRole("alert")).toContainText("A reset was requested");
  await page.getByRole("button", { name: "Cancel reset" }).click();
  await expect(page.getByRole("button", { name: "Cancel reset" })).toHaveCount(
    0,
  );
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter(
        (c) =>
          c.command === "sync_command" && c.args.operation === "reset_cancel",
      ),
    ),
  ).toHaveLength(1);
});

for (const signoutOnly of [false, true]) {
  test(`M1 removing another device with sign-out-only=${signoutOnly}`, async ({
    page,
  }) => {
    await installDesktopFixture(page, {
      syncEnabled: true,
      syncStatus: {
        signedIn: true,
        unlocked: true,
        email: "fixture@example.invalid",
        device: "self",
        server: "https://sync.example.invalid",
      },
    });
    await page.goto("/?page=settings");
    await page
      .getByRole("button", { name: "Account and sync", exact: false })
      .click();
    await page
      .getByRole("button", { name: "Account options", exact: true })
      .click();
    await page.getByRole("button", { name: "Remove", exact: true }).click();
    await expect(page.getByLabel("Sign out only, no re-key")).not.toBeChecked();
    if (signoutOnly) {
      await page.getByLabel("Sign out only, no re-key").check();
      await expect(page.getByRole("dialog")).toContainText(
        "can regain access by signing in again",
      );
    }
    await page
      .getByRole("button", { name: "Remove device", exact: true })
      .click();
    expect(
      await page.evaluate(() =>
        window.__testCalls
          .filter(
            (c) =>
              c.command === "sync_command" && c.args.operation === "remove",
          )
          .map((c) => c.args.args),
      ),
    ).toEqual([
      { device: "fixture-device", rekey: !signoutOnly, confirmed: true },
    ]);
  });
}
test("M1 removing this device only signs it out", async ({ page }) => {
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: {
      signedIn: true,
      unlocked: true,
      email: "fixture@example.invalid",
      device: "fixture-device",
      server: "https://sync.example.invalid",
    },
  });
  await page.goto("/?page=settings");
  await page
    .getByRole("button", { name: "Account and sync", exact: false })
    .click();
  await page
    .getByRole("button", { name: "Account options", exact: true })
    .click();
  await page.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(page.getByLabel("Sign out only, no re-key")).toHaveCount(0);
  await page
    .getByRole("button", { name: "Remove device", exact: true })
    .click();
  expect(
    await page.evaluate(() =>
      window.__testCalls
        .filter(
          (c) => c.command === "sync_command" && c.args.operation === "remove",
        )
        .map((c) => c.args.args.rekey),
    ),
  ).toEqual([false]);
  await expect(
    page.getByRole("button", { name: "Add this computer" }),
  ).toBeVisible();
  expect(await operations(page, "recovery_show")).toHaveLength(0);
});
test("L4 reset completion waits for ready_at and shows its availability", async ({
  page,
}) => {
  const start = new Date("2026-11-08T12:00:00Z");
  await page.clock.install({ time: start });
  const ready = start.getTime() / 1000 + 60;
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: {
      signedIn: true,
      unlocked: false,
      email: "fixture@example.invalid",
      device: "b",
      server: "https://sync.example.invalid",
      pendingReset: ready,
    },
  });
  await page.goto("/?page=settings");
  await page
    .getByRole("button", { name: "Account and sync", exact: false })
    .click();
  await page
    .getByRole("button", { name: "Can't approve from another computer?" })
    .click();
  const button = page.getByRole("button", {
    name: "Complete reset after 72 hours",
  });
  await expect(button).toBeDisabled();
  await expect(page.getByText("Available ", { exact: false })).toContainText(
    await page.evaluate((t) => new Date(t * 1000).toLocaleString(), ready),
  );
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter(
        (c) => c.command === "sync_command" && c.args.operation === "reset",
      ),
    ),
  ).toHaveLength(0);
  await page.clock.fastForward(61000);
  await expect(button).toBeEnabled();
  await button.click();
  await page.getByRole("button", { name: "Confirm", exact: true }).click();
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter(
        (c) => c.command === "sync_command" && c.args.operation === "reset",
      ),
    ),
  ).toHaveLength(1);
});

test("first account creates keys and saves recovery on one screen", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: {
      signedIn: true,
      unlocked: false,
      initialized: false,
      email: "fixture@example.invalid",
      device: "fixture-device",
      server: "https://sync.example.invalid",
    },
  });
  await page.goto("/?page=sync&flavor=synctest");
  await expect(
    page.getByRole("button", { name: "Compare six digits" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Create encryption key" }).click();
  await expect(page.getByText("SYNTHETIC-RECOVERY-FIXTURE")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Sync now", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Done", exact: true }),
  ).toBeDisabled();
  await page.getByLabel("I saved my recovery code").check();
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Sync now", exact: true }),
  ).toBeVisible();
  await expect(page.getByText("SYNTHETIC-RECOVERY-FIXTURE")).toHaveCount(0);
});
