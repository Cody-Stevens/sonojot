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

test("second computer waits for its key with nothing to compare or approve", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncSigninStatus: locked,
  });
  await page.goto("/?page=sync&flavor=synctest&theme=fog");
  await page.getByRole("button", { name: "Add this computer" }).click();
  await expect(
    page.getByRole("heading", {
      name: "Getting your key from your other computer",
    }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Keep SonoJot open on one of your other computers. This happens on its own.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("status").filter({ hasText: "Waiting…" }),
  ).toBeVisible();
  await expect(page.locator(".spinner")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Use recovery code instead" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Sign out", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".pair-code")).toHaveCount(0);
  for (const name of [
    "Approve",
    "Deny",
    "Cancel",
    "They match",
    "Get a new code",
    "Account options",
    "Download account data",
    "Delete account",
  ])
    await expect(page.getByRole("button", { name, exact: true })).toHaveCount(
      0,
    );
  await expect(page.locator(".sync-sheet")).toHaveCount(1);
  await expect
    .poll(async () => (await operations(page, "pair_wait")).length)
    .toBeGreaterThan(0);
  await screenshot(page, "new-computer");
});

test("sign-in callback starts waiting without reloading Settings", async ({
  page,
}) => {
  await installDesktopFixture(page, { syncEnabled: true });
  await page.goto("/?page=sync&flavor=synctest");
  await expect(
    page.getByRole("button", { name: "Add this computer" }),
  ).toBeVisible();
  await page.evaluate((status) => window.__testSyncChange({ status }), locked);
  await expect(
    page.getByRole("heading", {
      name: "Getting your key from your other computer",
    }),
  ).toBeVisible();
});

test("waiting computer keeps asking every few seconds and unlocks on its own", async ({
  page,
}) => {
  await page.clock.install();
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: locked,
    syncWaits: [{ waiting: true }, { waiting: true }, { unlocked: true }],
  });
  await page.goto("/?page=sync&flavor=synctest");
  await expect
    .poll(async () => (await operations(page, "pair_wait")).length)
    .toBe(1);
  await page.clock.fastForward(3100);
  await expect
    .poll(async () => (await operations(page, "pair_wait")).length)
    .toBe(2);
  await page.clock.fastForward(3100);
  await expect(
    page.getByRole("button", { name: "Sync now", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", {
      name: "Getting your key from your other computer",
    }),
  ).toHaveCount(0);
});

test("a failed check shows the reason and keeps waiting", async ({ page }) => {
  await page.clock.install();
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: locked,
    syncPairError:
      "Sync is offline. Your local words are safe; try Sync now when connected.",
  });
  await page.goto("/?page=sync&flavor=synctest");
  await expect(
    page.getByText("Sync is offline.", { exact: false }),
  ).toBeVisible();
  await expect(
    page.getByRole("status").filter({ hasText: "Waiting…" }),
  ).toBeVisible();
  // It backs off instead of retrying every poll, then resumes on its own.
  await page.clock.fastForward(9000);
  expect(await operations(page, "pair_wait")).toHaveLength(1);
  await page.clock.fastForward(15000);
  await expect
    .poll(async () => (await operations(page, "pair_wait")).length)
    .toBe(2);
  await expect(
    page.getByText("Sync is offline.", { exact: false }),
  ).toHaveCount(0);
});

const joined = {
  id: "mac",
  platform: "macos",
  joinedAt: Math.floor(new Date("2026-11-13T15:04:00Z").getTime() / 1000),
};

test("trusted computer shows a join alert and OK dismisses it", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: { ...trusted, joins: [joined] },
  });
  await page.goto("/?page=sync&flavor=synctest&theme=fog");
  const alert = page.getByRole("region", { name: "Computer joined" });
  await expect(
    alert.getByRole("heading", { name: "Mac joined sync" }),
  ).toBeVisible();
  await expect(alert).toContainText(
    await page.evaluate(
      (t) =>
        new Date(t * 1000).toLocaleString(undefined, {
          dateStyle: "medium",
          timeStyle: "short",
        }),
      joined.joinedAt,
    ),
  );
  for (const name of ["Approve", "Deny"])
    await expect(page.getByRole("button", { name, exact: true })).toHaveCount(
      0,
    );
  await screenshot(page, "join-alert");
  await alert.getByRole("button", { name: "OK", exact: true }).click();
  expect(await operations(page, "join_ack")).toEqual([{ device: "mac" }]);
  await expect(alert).toHaveCount(0);
  expect(await operations(page, "remove")).toHaveLength(0);
});

test("Not me confirms, removes that computer and changes the key", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: { ...trusted, joins: [joined] },
  });
  await page.goto("/?page=sync&flavor=synctest&theme=fog");
  await page.getByRole("button", { name: "Not me", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(
    dialog.getByRole("heading", {
      name: "Remove this computer and change the key?",
    }),
  ).toBeVisible();
  await expect(dialog.getByLabel("Sign out only, no re-key")).toHaveCount(0);
  await screenshot(page, "not-me");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await operations(page, "remove")).toHaveLength(0);
  await page.getByRole("button", { name: "Not me", exact: true }).click();
  await page
    .getByRole("button", { name: "Remove and change key", exact: true })
    .click();
  expect(await operations(page, "remove")).toEqual([
    { confirmed: true, device: "mac", rekey: true },
  ]);
  // The replacement recovery code follows, as for any re-key.
  await expect(page.getByText("SYNTHETIC-RECOVERY-FIXTURE")).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Computer joined" }),
  ).toHaveCount(0);
});

test("join alerts appear while Settings stays open", async ({ page }) => {
  await installDesktopFixture(page, { syncEnabled: true, syncStatus: trusted });
  await page.goto("/?page=sync&flavor=synctest");
  await expect(
    page.getByRole("button", { name: "Account options" }),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Computer joined" }),
  ).toHaveCount(0);
  await page.evaluate(
    (joins) => window.__testSyncChange({ status: { joins } }),
    [{ ...joined, platform: "windows", id: "pc" }],
  );
  await expect(
    page.getByRole("heading", { name: "Windows PC joined sync" }),
  ).toBeVisible();
});

test("recovery and reset options stay hidden until requested", async ({
  page,
}) => {
  await installDesktopFixture(page, { syncEnabled: true, syncStatus: locked });
  await page.goto("/?page=sync&flavor=synctest&theme=fog");
  await expect(
    page.getByRole("button", { name: "Sign out", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "I lost my devices and recovery code" }),
  ).toHaveCount(0);
  await expect(page.getByLabel("Recovery code", { exact: true })).toHaveCount(
    0,
  );
  const instead = page.getByRole("button", {
    name: "Use recovery code instead",
  });
  await expect(instead).toHaveAttribute("aria-expanded", "false");
  await instead.click();
  await expect(
    page.getByRole("button", { name: "I lost my devices and recovery code" }),
  ).toBeVisible();
  await screenshot(page, "recovery-instead");
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
  await page.getByRole("button", { name: "Use recovery code instead" }).click();
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
