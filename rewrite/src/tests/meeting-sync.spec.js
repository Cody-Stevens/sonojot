import { test, expect } from "@playwright/test";
import { openApp, openMeeting } from "./app-helpers.js";
const signedIn = {
  signedIn: true,
  unlocked: true,
  server: "https://sync.example.invalid",
  email: "synthetic@example.test",
  lastSync: 1794000000,
};
test("meeting audio stays opt-in and a per-meeting choice is explicit", async ({
  page,
}) => {
  await openApp(
    page,
    { syncEnabled: true, syncStatus: signedIn },
    "/?page=meetings",
  );
  await openMeeting(page, "Design review");
  await expect(page.getByLabel("Upload audio for this meeting")).toHaveValue(
    "default",
  );
  await expect(page.getByLabel("Meeting sync")).toContainText("Default (off)");
  await page.getByLabel("Upload audio for this meeting").selectOption("on");
  await expect(page.getByLabel("Upload audio for this meeting")).toHaveValue(
    "on",
  );
  await page.screenshot({ path: "artifacts/meeting-sync.png", fullPage: true });
});
test("meeting sync retains conflict choices and explains a deletion", async ({
  page,
}) => {
  const meetingSync = {
    status: "deletedElsewhere",
    audioOverride: null,
    audioEnabled: false,
    audioAvailable: false,
    deleteConflict: true,
    conflicts: [
      {
        sid: "line-one",
        view: "original",
        version: "synthetic",
        text: "Other synthetic wording.",
      },
    ],
    fieldConflicts: { "head/fields/title": "Other title" },
  };
  await openApp(
    page,
    { syncEnabled: true, syncStatus: signedIn, meetingSync },
    "/?page=meetings",
  );
  await openMeeting(page, "Design review");
  await page.getByText("Edited on two devices (1)", { exact: true }).click();
  await expect(page.getByText("Other synthetic wording.")).toBeVisible();
  await page.getByRole("button", { name: "Use the other version" }).click();
  await expect(
    page.getByText("Edited on two devices (1)", { exact: true }),
  ).toHaveCount(0);
  await page
    .getByText("Meeting details changed on two devices", { exact: true })
    .click();
  await page.getByRole("button", { name: "Use this value" }).click();
  await expect(
    page.getByText("Meeting details changed on two devices", { exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Keep on this computer", exact: true })
    .click();
  await expect(page.getByLabel("Meeting sync")).toContainText(
    "Kept on this computer",
  );
});
test("storage shows the allowance and defaults audio uploads off", async ({
  page,
}) => {
  await openApp(
    page,
    { syncEnabled: true, syncStatus: signedIn },
    "/?page=settings",
  );
  await page
    .getByRole("button", { name: "Account and sync", exact: false })
    .click();
  await expect(
    page.getByText("Meeting storage: 1.0 MB of 100.0 GB", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByLabel("Upload meeting audio by default"),
  ).not.toBeChecked();
  await expect(
    page.getByText("Audio uses about 0.5 MB per minute", { exact: false }),
  ).toBeVisible();
  await page.getByLabel("Always download synced audio").click();
  await expect(page.getByLabel("Always download synced audio")).toBeChecked();
  await page.screenshot({
    path: "artifacts/meeting-sync-storage.png",
    fullPage: true,
  });
});

test("stable builds never show or request meeting sync", async ({ page }) => {
  await openApp(
    page,
    { syncEnabled: false, syncStatus: signedIn },
    "/?page=meetings",
  );
  await openMeeting(page, "Design review");
  await expect(page.getByLabel("Meeting sync")).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter((c) => c.command === "sync_command"),
    ),
  ).toHaveLength(0);
});

for (const quota of [0, 2_000_000_000]) {
  test(`storage displays the verified allowance: ${quota}`, async ({
    page,
  }) => {
    await openApp(
      page,
      { syncEnabled: true, syncStatus: signedIn, meetingQuota: quota },
      "/?page=settings",
    );
    await page
      .getByRole("button", { name: "Account and sync", exact: false })
      .click();
    await expect(
      page.getByText(
        quota
          ? "Meeting storage: 1.0 MB of 2.0 GB"
          : "Meeting storage: 1.0 MB. Sync to check your allowance.",
        { exact: true },
      ),
    ).toBeVisible();
    await expect(page.getByText(/100.0 GB/)).toHaveCount(0);
  });
}

test("recently deleted meetings can be restored or permanently deleted", async ({
  page,
}) => {
  await openApp(
    page,
    {
      syncEnabled: true,
      syncStatus: signedIn,
      meetingTrash: [
        {
          key: "1794000000-synthetic-a",
          id: "restored-fixture",
          title: "Synthetic saved meeting",
          deletedAt: 1794000000,
        },
        {
          key: "1794000000-synthetic-b",
          id: "removed-fixture",
          title: "Synthetic discarded meeting",
          deletedAt: 1794000000,
        },
      ],
    },
    "/?page=meetings",
  );
  await page
    .getByRole("button", { name: "Recently deleted", exact: true })
    .click();
  const trash = page.getByRole("region", { name: "Recently deleted meetings" });
  await expect(trash).toContainText("Kept for 30 days");
  await trash
    .getByRole("button", { name: "Restore", exact: true })
    .first()
    .click();
  await expect(trash.getByText("Synthetic saved meeting")).toHaveCount(0);
  await expect(
    page.getByText("Synthetic saved meeting", { exact: true }),
  ).toBeVisible();
  await trash.getByRole("button", { name: "Delete now", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("permanently removed");
  await dialog.getByRole("button", { name: "Delete now", exact: true }).click();
  await expect(trash).toContainText("No recently deleted meetings");
});
test("delete confirmation explains deletion on all synced devices", async ({
  page,
}) => {
  await openApp(
    page,
    { syncEnabled: true, syncStatus: signedIn },
    "/?page=meetings",
  );
  await openMeeting(page, "Design review");
  await page
    .getByRole("button", { name: "Meeting options", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Meeting", exact: true })
    .getByRole("button", { name: /^Delete/, exact: false })
    .click();
  await expect(page.getByRole("dialog")).toContainText(
    "deleted on all synced devices",
  );
});

for (const remove of [false, true]) {
  test(`missing-meeting notice requires an explicit deletion choice: ${remove}`, async ({
    page,
  }) => {
    await openApp(
      page,
      {
        syncEnabled: true,
        syncStatus: {
          ...signedIn,
          meetings: { missing: 8, errors: 0, retryAt: 0, rateLimited: false },
        },
      },
      "/?page=meetings",
    );
    const notice = page.getByRole("region", { name: "Missing meetings" });
    await expect(notice).toContainText(
      "8 meetings are missing on this computer",
    );
    await expect(notice.getByRole("button").first()).toHaveText(
      "Keep them on other devices",
    );
    await notice
      .getByRole("button", {
        name: remove ? "Delete them everywhere" : "Keep them on other devices",
        exact: true,
      })
      .click();
    if (remove) {
      const dialog = page.getByRole("dialog");
      await expect(dialog).toContainText("all synced devices");
      await dialog
        .getByRole("button", { name: "Delete them everywhere", exact: true })
        .click();
    }
    await expect(notice).toHaveCount(0);
    const calls = await page.evaluate(() =>
      window.__testCalls.filter(
        (c) =>
          c.command === "sync_command" &&
          c.args.operation === "meeting_missing_resolve",
      ),
    );
    expect(calls).toHaveLength(1);
    expect(calls[0].args.args.delete).toBe(remove);
  });
}
test("Sync now displays a partial meeting result", async ({ page }) => {
  await openApp(
    page,
    {
      syncEnabled: true,
      syncStatus: signedIn,
      meetingResult: { errors: 1, retryAt: 0, rateLimited: false, missing: 0 },
    },
    "/?page=settings",
  );
  await page
    .getByRole("button", { name: "Account and sync", exact: false })
    .click();
  await page.getByRole("button", { name: "Sync now", exact: true }).click();
  await expect(
    page
      .getByRole("status")
      .filter({ hasText: "1 meeting sync items need another try" }),
  ).toBeVisible();
});
