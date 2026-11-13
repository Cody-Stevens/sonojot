import { test, expect } from "@playwright/test";
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
  await expect(
    page.getByRole("button", { name: "Continue with WorkOS AuthKit" }),
  ).toBeVisible();
  await page.screenshot({
    path: "artifacts/sync-test-flavor.png",
    fullPage: true,
  });
});
test("account screen offers WorkOS AuthKit and an actionable failure", async ({
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
  await page
    .getByRole("button", { name: "Continue with WorkOS AuthKit" })
    .click();
  expect(
    await page.evaluate(() =>
      window.__testCalls
        .filter(
          (c) => c.command === "sync_command" && c.args.operation === "signin",
        )
        .map((c) => c.args.args.provider),
    ),
  ).toEqual(["authkit"]);
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

test("receiving device waits for its own code confirmation", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    syncEnabled: true,
    syncStatus: {
      signedIn: true,
      unlocked: false,
      email: "fixture@example.invalid",
      device: "b",
      server: "https://sync.example.invalid",
    },
  });
  await page.goto("/?page=settings");
  await page
    .getByRole("button", { name: "Account and sync", exact: false })
    .click();
  await page
    .getByRole("button", { name: "Approve on another device", exact: true })
    .click();
  await page.getByRole("button", { name: "Check pairing" }).click();
  await expect(page.locator(".pair-code")).toHaveText("654321");
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter(
        (c) =>
          c.command === "sync_command" && c.args.operation === "pair_confirm",
      ),
    ),
  ).toHaveLength(0);
  await page.getByRole("button", { name: "They match" }).click();
  expect(
    await page.evaluate(() =>
      window.__testCalls
        .filter(
          (c) =>
            c.command === "sync_command" && c.args.operation === "pair_confirm",
        )
        .map((c) => c.args.args),
    ),
  ).toEqual([{ id: "fixture-pair", confirmed: true }]);
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
    await page.getByRole("button", { name: "Remove", exact: true }).click();
    await expect(page.getByLabel("Sign out only, no re-key")).not.toBeChecked();
    if (signoutOnly) {
      await page.getByLabel("Sign out only, no re-key").check();
      await expect(page.getByRole("dialog")).toContainText(
        "can regain access by signing in again",
      );
    }
    await page.getByRole("button", { name: "Confirm", exact: true }).click();
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
  await page.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(page.getByLabel("Sign out only, no re-key")).toHaveCount(0);
  await page.getByRole("button", { name: "Confirm", exact: true }).click();
  expect(
    await page.evaluate(() =>
      window.__testCalls
        .filter(
          (c) => c.command === "sync_command" && c.args.operation === "remove",
        )
        .map((c) => c.args.args.rekey),
    ),
  ).toEqual([false]);
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
