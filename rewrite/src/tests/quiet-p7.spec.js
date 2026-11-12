import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture } from "./desktop-fixture.js";

const folder = new URL("../../artifacts/redesign-p7/", import.meta.url);
const transcript =
  "[0:01] Speaker 1: The launch is Friday.\n\n[0:05] Speaker 2: Rachel will prepare the Kubernetes migration.\n\n[0:12] Speaker 2: Keep the backup.";
const raw =
  "the launch is friday rachel will prepare cooper netties keep the backup";
const calls = (page, command) =>
  page.evaluate(
    (command) => window.__testCalls.filter((call) => call.command === command),
    command,
  );
const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
async function open(page, pageId = "meetings", extra = {}) {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await installDesktopFixture(page, {
    ...extra,
    settings: {
      setupProfile: "balanced",
      meetingConsentAcknowledged: true,
      ...extra.settings,
    },
    meeting: {
      transcript,
      cleanedTranscript: transcript,
      rawTranscript: raw,
      notes: "# Decisions\nPrepare the Kubernetes migration.",
      speakerCount: 2,
      ...extra.meeting,
    },
  });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(`/?page=${pageId}`);
  await expect(
    page.getByRole("navigation", { name: "Main navigation" }),
  ).toBeVisible();
  return errors;
}
async function shot(page, name) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(name, folder)) });
}
async function openMeeting(page) {
  await page.getByRole("button", { name: /Design review/ }).click();
  await page.getByRole("tab", { name: "Transcript", exact: true }).click();
}

test("passage assignment changes one passage, supports new and existing names, and leaves recognition intact", async ({
  page,
}) => {
  const errors = await open(page);
  await openMeeting(page);
  await page
    .getByRole("button", {
      name: "Rachel will prepare the Kubernetes migration.",
      exact: true,
    })
    .click();
  await page
    .getByRole("button", { name: "Assign passage…", exact: true })
    .click();
  const sheet = page.getByRole("dialog", { name: "Assign this passage" });
  await sheet.getByRole("button", { name: "New name…", exact: true }).click();
  await sheet.getByRole("textbox", { name: "New speaker name" }).fill("Rachel");
  await shot(page, "01-assign-passage.png");
  await sheet
    .getByRole("button", { name: "Assign passage", exact: true })
    .click();
  await expect(sheet).toBeHidden();
  expect((await calls(page, "reassign_meeting_passage"))[0].args).toEqual({
    id: "meeting-one",
    clock: "0:05",
    from: "Speaker 2",
    to: "Rachel",
  });
  await expect(
    page.locator(".passage-who").filter({ hasText: "Rachel" }),
  ).toHaveCount(1);
  await expect(
    page.locator(".passage-who").filter({ hasText: "Speaker 2" }),
  ).toHaveCount(1);
  await page
    .getByRole("button", {
      name: "Rachel will prepare the Kubernetes migration.",
      exact: true,
    })
    .click();
  await page
    .getByRole("textbox", { name: "Speaker name", exact: true })
    .fill("Rachel Green");
  await page.getByRole("button", { name: "Rename everywhere" }).click();
  await expect(
    page.locator(".passage-who").filter({ hasText: "Rachel Green" }),
  ).toHaveCount(1);
  await page
    .getByRole("button", { name: "Keep the backup.", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Assign passage…", exact: true })
    .click();
  await sheet
    .getByRole("button", { name: "Rachel Green", exact: true })
    .click();
  await sheet
    .getByRole("button", { name: "Assign passage", exact: true })
    .click();
  await expect(sheet).toBeHidden();
  await shot(page, "02-reassigned-transcript.png");
  await page.getByRole("button", { name: "Meeting options" }).click();
  await page
    .getByRole("button", { name: "Transcript version…", exact: true })
    .click();
  await page.getByRole("button", { name: "As heard", exact: true }).click();
  await expect(page.locator(".raw-transcript")).toHaveText(raw);
  expect(errors).toEqual([]);
});

test("Words stores hints separately from exact replacements and only shows model guidance for rules", async ({
  page,
}) => {
  const errors = await open(page, "vocabulary", {
    settings: {
      localProofreader: false,
      proofreader: false,
      cleanupModel: "local-rules",
    },
  });
  const add = page.getByRole("textbox", { name: "Add a word" });
  await add.fill("Kubernetes");
  await add.press("Enter");
  await expect(
    page.getByRole("button", { name: "Kubernetes hint", exact: true }),
  ).toContainText("HINT");
  await add.fill("omar key -> Omarchy");
  await add.press("Enter");
  await expect(
    page.getByRole("button", { name: "omar key becomes Omarchy" }),
  ).toBeVisible();
  let entries = (await calls(page, "save_vocabulary")).at(-1).args.entries;
  expect(entries[0]).toMatchObject({
    kind: "hint",
    heard: "Kubernetes",
    written: "Kubernetes",
    aliases: [],
  });
  expect(entries[1]).toMatchObject({
    kind: "replace",
    heard: "omar key",
    aliases: ["omar key"],
  });
  await expect(
    page.getByText("Hints help when a model does the cleanup."),
  ).toBeVisible();
  await shot(page, "03-words-hints.png");
  await page
    .getByRole("button", { name: "Kubernetes hint", exact: true })
    .click();
  await expect(
    page.getByRole("textbox", { name: "Heard", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("textbox", { name: "Written", exact: true })
    .fill("Kubernetes CLI");
  await page.getByRole("button", { name: "Done", exact: true }).click();
  entries = (await calls(page, "save_vocabulary")).at(-1).args.entries;
  expect(entries[0]).toMatchObject({
    kind: "hint",
    heard: "Kubernetes CLI",
    written: "Kubernetes CLI",
  });
  await page.evaluate(() => {
    const next = window.__testSnapshot().settings;
    window.__testPushSnapshot({ settings: { ...next, proofreader: true } });
  });
  await expect(
    page.getByText("Hints help when a model does the cleanup."),
  ).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("Notes defaults persist No notes and custom instructions with a draft guard", async ({
  page,
}) => {
  const errors = await open(page, "meetings", {
    settings: { meetingNotesPreset: "summary" },
  });
  await page.getByRole("button", { name: "Notes", exact: true }).click();
  const style = page.getByLabel("Style", { exact: true });
  await expect(style).toHaveValue("summary");
  await style.selectOption("");
  await expect
    .poll(async () => (await settings(page)).meetingNotesPreset)
    .toBeNull();
  await shot(page, "04-no-notes.png");
  await style.selectOption("custom");
  await page
    .getByLabel("Instructions", { exact: true })
    .fill("List decisions, owners and open questions.");
  await page.locator('[data-nav="words"]').click();
  await page.getByRole("button", { name: "Keep working", exact: true }).click();
  await page.getByRole("button", { name: "Save instructions" }).click();
  await expect
    .poll(async () => (await settings(page)).meetingNotesInstructions)
    .toBe("List decisions, owners and open questions.");
  expect(errors).toEqual([]);
});

test("Stop opens processing and then saved notes without a manual notes request", async ({
  page,
}) => {
  const errors = await open(page, "meetings", {
    settings: { meetingNotesPreset: "summary" },
  });
  await page
    .getByRole("button", { name: "Record meeting", exact: false })
    .click();
  await page.getByRole("button", { name: /Stop$/, exact: false }).click();
  await page.evaluate(
    (transcript) =>
      window.__testUpdateMeetingById("new-meeting", {
        status: "processingNotes",
        transcript,
        hasTranscript: true,
      }),
    transcript,
  );
  await expect(
    page.getByRole("region", { name: "Meeting progress" }),
  ).toBeVisible();
  await shot(page, "05-writing-notes.png");
  await page.evaluate(() =>
    window.__testUpdateMeetingById("new-meeting", {
      status: "processed",
      notes:
        "# Summary\n- Prepare the Kubernetes migration.\n\n# Next steps\n- Rachel will prepare the plan.",
    }),
  );
  await expect(
    page.getByRole("tab", { name: "Notes", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("button", { name: "Edit notes" })).toContainText(
    "Kubernetes",
  );
  expect(await calls(page, "generate_notes")).toHaveLength(0);
  await shot(page, "06-notes-after-stop.png");
  expect(errors).toEqual([]);
});

test("meeting search finds notes and published text and displays the matching line", async ({
  page,
}) => {
  const errors = await open(page, "meetings", {
    meeting: {
      notes: "# Decisions\nBudget approved for Northwind.",
      transcript,
    },
  });
  const search = page.getByRole("textbox", { name: "Search meetings" });
  await search.fill("Northwind");
  await expect(page.locator(".meeting-slip")).toHaveCount(1);
  await expect(page.locator(".search-match")).toHaveText(
    "Budget approved for Northwind.",
  );
  await shot(page, "07-search-notes.png");
  await search.fill("Kubernetes");
  await expect(page.locator(".search-match")).toContainText(
    "Rachel will prepare",
  );
  await search.fill("cooper netties");
  await expect(
    page.getByText("No matching meetings.", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Clear search" }).click();
  await expect(page.locator(".meeting-slip")).toHaveCount(2);
  expect(errors).toEqual([]);
});
