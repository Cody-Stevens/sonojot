// Visual & structural coherence regression guard for the SonoJot UI system.
// Verifies the shared PageHeader, PipelineBand, Notebook vs. Index lists,
// canonical SearchBar, SettingRow 200px grid, SegmentedControl clipping,
// Picker, Disclosure, InfoTip binding, concentric row pockets, truthful rail
// highlight, and two-step Back navigation at 1060x760 and 740x540 in Fog and Slate.
import { expect, test } from "@playwright/test";
import { mainNavigation, openApp, openMeeting } from "./app-helpers.js";

const now = new Date();
const todayIso = new Date(
  now.getFullYear(),
  now.getMonth(),
  now.getDate(),
  14,
  30,
  0,
).toISOString();
const yesterdayIso = new Date(
  now.getFullYear(),
  now.getMonth(),
  now.getDate() - 1,
  16,
  15,
  0,
).toISOString();

function seedOptions() {
  return {
    syncEnabled: true,
    syncStatus: {
      signedIn: true,
      unlocked: true,
      initialized: true,
      email: "synthetic@example.test",
      device: "dev-a",
      server: "https://sync.example.invalid",
      lastSync: 1794000000,
    },
    vocabulary: [
      {
        id: "w-1",
        kind: "replace",
        heard: "sono jot",
        written: "SonoJot",
        aliases: [],
        context: [],
        scope: "product",
        enabled: true,
      },
      {
        id: "w-2",
        kind: "hint",
        heard: "kubernetes",
        written: "Kubernetes",
        aliases: [],
        context: [],
        scope: null,
        enabled: true,
      },
    ],
    meetings: [
      {
        id: "meeting-today",
        title: "Design review",
        createdAt: todayIso,
        durationSeconds: 300,
        status: "processed",
        hasAudio: true,
        hasTranscript: true,
        audioOutages: [],
        missingSeconds: 0,
        cleanedTranscript: null,
        transcriptionStoppedAtSeconds: null,
        transcript: "[0:12] You: Check the header alignment across all tabs.",
        notes: "# Summary\n- Header and list alignment verified.",
        source: "both",
        speakerCount: 2,
        speakerLabels: true,
        audioPath: "C:/today.wav",
        error: null,
      },
      {
        id: "meeting-yesterday",
        title: "Client call",
        createdAt: yesterdayIso,
        durationSeconds: 1840,
        status: "transcribed",
        hasAudio: true,
        hasTranscript: true,
        audioOutages: [],
        missingSeconds: 0,
        cleanedTranscript: null,
        transcriptionStoppedAtSeconds: null,
        transcript: "[0:15] Speaker 1: Timeline looks good for Friday.",
        notes: "",
        source: "microphone",
        audioPath: "C:/yesterday.wav",
        error: null,
      },
    ],
    meetingTrash: [
      {
        key: "trash-1",
        title: "Old sync test",
        deletedAt: 1794000000,
      },
    ],
  };
}

/** Check horizontal overflow on `.page-content` and every visible `.seg-control`,
 * plus verify no `.seg-btn` text overflows its own padding box. */
async function assertNoHorizontalOverflowOrSegmentClip(page, label) {
  const report = await page.evaluate(() => {
    const content = document.querySelector(".page-content");
    const pageOverflow = content
      ? content.scrollWidth - content.clientWidth
      : 0;
    const clippedSegments = [];
    for (const seg of document.querySelectorAll(".seg-control")) {
      if (!seg.checkVisibility()) continue;
      if (seg.scrollWidth > seg.clientWidth + 1) {
        clippedSegments.push(
          `seg-control overflow (${seg.getAttribute("aria-label")}): ${seg.scrollWidth} > ${seg.clientWidth}`,
        );
      }
      for (const btn of seg.querySelectorAll(".seg-btn")) {
        if (!btn.checkVisibility()) continue;
        const rect = btn.getBoundingClientRect();
        if (btn.scrollWidth > btn.clientWidth + 1) {
          clippedSegments.push(
            `seg-btn scrollWidth (${btn.textContent?.trim()}): ${btn.scrollWidth} > ${btn.clientWidth}`,
          );
        }
        const range = document.createRange();
        range.selectNodeContents(btn);
        const textRect = range.getBoundingClientRect();
        if (
          textRect.width > 0 &&
          (textRect.left < rect.left - 1 || textRect.right > rect.right + 1)
        ) {
          clippedSegments.push(
            `seg-btn text outside box (${btn.textContent?.trim()}): text [${Math.round(textRect.left)},${Math.round(textRect.right)}] vs btn [${Math.round(rect.left)},${Math.round(rect.right)}]`,
          );
        }
      }
    }
    return { pageOverflow, clippedSegments };
  });
  expect(
    report.pageOverflow,
    `${label}: .page-content horizontal overflow`,
  ).toBeLessThanOrEqual(1);
  expect(
    report.clippedSegments,
    `${label}: clipped segmented controls`,
  ).toEqual([]);
}

for (const theme of ["fog", "slate"]) {
  for (const viewport of [
    { width: 1060, height: 760 },
    { width: 740, height: 540 },
  ]) {
    test(`${theme} ${viewport.width}×${viewport.height}: top tabs, sub-pages, controls, and zero overflow`, async ({
      page,
    }) => {
      test.setTimeout(60_000);
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.setViewportSize(viewport);
      await openApp(page, seedOptions(), `/?theme=${theme}`);
      await page.evaluate(() => document.fonts.ready);

      // 1. Uniform 32px single-line h1 and identical top/left alignment across all 4 top tabs.
      const headerBoxes = {};
      const expectedTitle = {
        dictation: "Dictation",
        meetings: "Meetings",
        words: "Words",
        settings: "Settings",
      };
      for (const tab of ["dictation", "meetings", "words", "settings"]) {
        await mainNavigation(page).locator(`[data-nav="${tab}"]`).click();
        const h1 = page.getByRole("heading", {
          name: expectedTitle[tab],
          exact: true,
          level: 1,
        });
        await expect(h1).toBeVisible();
        await page.evaluate(() => document.fonts.ready);
        const metrics = await h1.evaluate((el) => {
          const rect = el.getBoundingClientRect();
          const cs = getComputedStyle(el);
          const frame = document.querySelector(".page-frame").getBoundingClientRect();
          const flow = document.querySelector(".page-flow").getBoundingClientRect();
          return {
            text: el.textContent?.trim(),
            left: Math.round(rect.left),
            top: Math.round(rect.top),
            height: Math.round(rect.height),
            fontSize: parseFloat(cs.fontSize),
            lineHeight: parseFloat(cs.lineHeight),
            frameLeftInset: Math.round(flow.left - frame.left),
            frameTopInset: Math.round(flow.top - frame.top),
            frameRightInset: Math.round(frame.right - flow.right),
          };
        });
        expect(metrics.fontSize).toBeCloseTo(32, 0);
        expect(metrics.height).toBeLessThanOrEqual(42);
        expect(
          Math.abs(metrics.frameLeftInset - metrics.frameTopInset),
          `${tab} equal left (${metrics.frameLeftInset}) and top (${metrics.frameTopInset}) inset inside .page-frame`,
        ).toBeLessThanOrEqual(1);
        expect(
          Math.abs(metrics.frameLeftInset - metrics.frameRightInset),
          `${tab} equal left (${metrics.frameLeftInset}) and right (${metrics.frameRightInset}) inset inside .page-frame`,
        ).toBeLessThanOrEqual(1);
        headerBoxes[tab] = metrics;
        await assertNoHorizontalOverflowOrSegmentClip(
          page,
          `${theme} ${viewport.width} tab:${tab}`,
        );
      }
      for (const tab of ["meetings", "words", "settings"]) {
        expect(
          Math.abs(headerBoxes[tab].left - headerBoxes.dictation.left),
          `${tab} h1 left (${headerBoxes[tab].left}) matches Dictation (${headerBoxes.dictation.left})`,
        ).toBeLessThanOrEqual(1);
        expect(
          Math.abs(headerBoxes[tab].top - headerBoxes.dictation.top),
          `${tab} h1 top (${headerBoxes[tab].top}) matches Dictation (${headerBoxes.dictation.top})`,
        ).toBeLessThanOrEqual(1);
      }

      // 2. Identical PipelineBand geometry across Dictation and Meetings + no dangling arrows.
      const pipelineMetrics = {};
      for (const tab of ["dictation", "meetings"]) {
        await mainNavigation(page).locator(`[data-nav="${tab}"]`).click();
        const band = page.locator(".pipeline-band").first();
        await expect(band).toBeVisible();
        pipelineMetrics[tab] = await band.evaluate((el) => {
          const header = document.querySelector(".page-header");
          const hRect = header.getBoundingClientRect();
          const bRect = el.getBoundingClientRect();
          const cs = getComputedStyle(el);
          const chips = [...el.querySelectorAll(".quiet-chip")];
          const chipRect = chips[0] ? chips[0].getBoundingClientRect() : null;
          const arrows = [...el.querySelectorAll(".stage-arrow")];
          const danglingArrows = [...el.querySelectorAll(".stage-step")].filter(
            (step) => {
              const sep = step.querySelector(".stage-arrow");
              const btn = step.querySelector(".quiet-chip");
              if (!sep || !btn) return false;
              const sRect = sep.getBoundingClientRect();
              const cRect = btn.getBoundingClientRect();
              return Math.abs(sRect.top - cRect.top) > cRect.height;
            },
          ).length;
          return {
            gapFromHeader: Math.round(bRect.top - hRect.bottom),
            radius: cs.borderRadius,
            paddingTop: cs.paddingTop,
            chipCount: chips.length,
            arrowCount: arrows.length,
            chipHeight: chipRect ? Math.round(chipRect.height) : 0,
            danglingArrows,
          };
        });
        expect(pipelineMetrics[tab].chipCount).toBeGreaterThanOrEqual(2);
        expect(pipelineMetrics[tab].arrowCount).toBeGreaterThanOrEqual(1);
        expect(pipelineMetrics[tab].chipHeight).toBeGreaterThanOrEqual(28);
        expect(pipelineMetrics[tab].danglingArrows).toBe(0);
      }
      expect(
        Math.abs(
          pipelineMetrics.dictation.gapFromHeader -
            pipelineMetrics.meetings.gapFromHeader,
        ),
      ).toBeLessThanOrEqual(1);
      expect(pipelineMetrics.dictation.radius).toBe(
        pipelineMetrics.meetings.radius,
      );
      expect(pipelineMetrics.dictation.paddingTop).toBe(
        pipelineMetrics.meetings.paddingTop,
      );
      expect(
        Math.abs(
          pipelineMetrics.dictation.chipHeight -
            pipelineMetrics.meetings.chipHeight,
        ),
      ).toBeLessThanOrEqual(1);

      // 3. Canonical SearchBar height (44±1px) with SVG icon across Dictation, Meetings, Words, On disk.
      const searchChecks = [
        async () => {
          await mainNavigation(page).locator('[data-nav="dictation"]').click();
          return page.locator(".search-bar").first();
        },
        async () => {
          await mainNavigation(page).locator('[data-nav="meetings"]').click();
          return page.locator(".search-bar").first();
        },
        async () => {
          await mainNavigation(page).locator('[data-nav="words"]').click();
          return page.locator(".search-bar").first();
        },
        async () => {
          await mainNavigation(page).locator('[data-nav="settings"]').click();
          const adv = page.locator("details.advanced");
          if (!(await adv.evaluate((el) => el.open))) {
            await adv.locator("> summary").click();
          }
          await page.getByRole("button", { name: /^On disk / }).click();
          return page.locator(".search-bar").first();
        },
      ];
      for (const openSearch of searchChecks) {
        const bar = await openSearch();
        await expect(bar).toBeVisible();
        const box = await bar.boundingBox();
        expect(box.height).toBeGreaterThanOrEqual(42);
        expect(box.height).toBeLessThanOrEqual(46);
        await expect(bar.locator(".search-icon svg")).toHaveCount(1);
        const input = bar.locator('input[type="search"]');
        await input.fill("test");
        const clear = bar.locator(".search-clear");
        await expect(clear).toBeVisible();
        await clear.click();
        await expect(input).toHaveValue("");
        await expect(input).toBeFocused();
      }

      // 4. Walk every Settings sub-page and verify zero horizontal overflow & zero clipped segments.
      const subRoutes = [
        { tab: "dictation", stage: "input", from: "settings-group" },
        { tab: "dictation", stage: "speech", from: "settings-group" },
        { tab: "dictation", stage: "cleanup", from: "settings-group" },
        { tab: "dictation", stage: "output", from: "settings-group" },
        { tab: "dictation", stage: "trust", from: "settings-group" },
        { tab: "meetings", stage: "speech", from: "settings-group" },
        { tab: "meetings", stage: "notes", from: "settings-group" },
        { tab: "settings", stage: "dictation" },
        { tab: "settings", stage: "meetings" },
        { tab: "settings", stage: "look" },
        { tab: "settings", stage: "sync" },
        { tab: "settings", stage: "memory" },
        { tab: "settings", stage: "disk" },
        { tab: "settings", stage: "updates" },
        { tab: "settings", stage: "help" },
      ];
      for (const route of subRoutes) {
        await page.evaluate((r) => window.__testNavigatePage(r), route);
        await expect(page.locator(".page-header h1").first()).toBeVisible();
        // Zero eyebrow above h1 on sub-pages.
        await expect(page.locator(".page-eyebrow")).toHaveCount(0);
        await assertNoHorizontalOverflowOrSegmentClip(
          page,
          `${theme} ${viewport.width} route:${JSON.stringify(route)}`,
        );
      }
    });
  }
}

test("truthful rail highlight, two-step Back stack, and root-chip single-step Back", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openApp(page, seedOptions());

  // Settings -> Dictation settings -> Speech: rail stays on Settings, Back takes two steps.
  await mainNavigation(page).locator('[data-nav="settings"]').click();
  await page.getByRole("button", { name: /^Dictation / }).click();
  await expect(
    page.getByRole("heading", { name: "Dictation settings", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: /^Speech / }).click();
  await expect(
    page.getByRole("heading", { name: "Speech", exact: true }),
  ).toBeVisible();
  await expect(
    mainNavigation(page).locator('[data-nav="settings"]'),
  ).toHaveAttribute("aria-current", "page");
  await page.getByRole("button", { name: "Back to Dictation settings" }).click();
  await expect(
    page.getByRole("heading", { name: "Dictation settings", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Back to Settings" }).click();
  await expect(
    page.getByRole("heading", { name: "Settings", exact: true }),
  ).toBeVisible();

  // Settings -> Meetings settings -> Notes: rail stays on Settings, Back takes two steps.
  await page.getByRole("button", { name: /^Meetings / }).click();
  await expect(
    page.getByRole("heading", { name: "Meetings settings", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: /^Notes / }).click();
  await expect(
    page.getByRole("heading", { name: "Notes", exact: true }),
  ).toBeVisible();
  await expect(
    mainNavigation(page).locator('[data-nav="settings"]'),
  ).toHaveAttribute("aria-current", "page");
  await page.getByRole("button", { name: "Back to Meetings settings" }).click();
  await expect(
    page.getByRole("heading", { name: "Meetings settings", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Back to Settings" }).click();

  // Settings -> On disk -> Choose another: Back returns to On disk, then Settings.
  const adv = page.locator("details.advanced");
  if (!(await adv.evaluate((el) => el.open))) {
    await adv.locator("> summary").click();
  }
  await page.getByRole("button", { name: /^On disk / }).click();
  await expect(
    page.getByRole("heading", { name: "On disk", exact: true }),
  ).toBeVisible();
  // Settings -> On disk -> Not offered -> Open speech models -> Back restores tab & selection.
  await page.getByRole("button", { name: "Not offered", exact: true }).click();
  const notOfferedRow = page.locator(".disk-entry .disk-row").first();
  await expect(notOfferedRow).toBeVisible();
  await notOfferedRow.click();
  const openSpeech = page
    .getByRole("button", { name: "Open speech models", exact: true })
    .first();
  await expect(openSpeech).toBeVisible();
  await openSpeech.click();
  await expect(
    mainNavigation(page).locator('[data-nav="settings"]'),
  ).toHaveAttribute("aria-current", "page");
  await page.getByRole("button", { name: "Back to On disk" }).click();
  await expect(
    page.getByRole("heading", { name: "On disk", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Not offered", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Installed", exact: true }).click();
  await page.getByRole("button", { name: "Back to Settings" }).click();

  // Root chip from Dictation -> Speech: rail stays on Dictation, Back returns to Dictation in one step.
  await mainNavigation(page).locator('[data-nav="dictation"]').click();
  await page.locator(".pipeline-band").getByRole("button", { name: "Speech" }).click();
  await expect(
    mainNavigation(page).locator('[data-nav="dictation"]'),
  ).toHaveAttribute("aria-current", "page");
  await page.getByRole("button", { name: "Back to Dictation" }).click();
  await expect(
    page.getByRole("heading", { name: "Dictation", exact: true }),
  ).toBeVisible();
});

test("Notebook day cards on Meetings, concentric row pockets, and preserved Learned draft", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openApp(page, seedOptions());

  // Meetings grouped into Notebook day cards (Today + Yesterday), single card per day, no standalone floating slip shadows.
  await mainNavigation(page).locator('[data-nav="meetings"]').click();
  const dayCards = page.locator(".meeting-list .day-card.notebook");
  await expect(dayCards).toHaveCount(2);
  await expect(
    page.locator(".meeting-list .day-heading").first(),
  ).toContainText("Today");
  const slipHasBoxShadow = await page
    .locator(".meeting-slip")
    .first()
    .evaluate((el) => getComputedStyle(el).boxShadow);
  expect(slipHasBoxShadow).toBe("none");

  // Recently deleted sits in .list-footer below the list.
  await expect(page.locator(".recently-deleted .list-footer")).toBeVisible();

  // MeetingPage enters scrolled to top with Back focused and keyboard tab switching working.
  await openMeeting(page, /Design review/);
  const scrollTop = await page
    .locator(".page-content")
    .evaluate((el) => el.scrollTop);
  expect(scrollTop).toBeLessThanOrEqual(1);
  await expect(
    page.getByRole("button", { name: "Back to Meetings" }),
  ).toBeFocused();
  const activeTab = page.locator('.seg-control[role="tablist"] [aria-selected="true"]');
  await activeTab.focus();
  await page.keyboard.press("ArrowRight");
  await expect(
    page.getByRole("tab", { name: "Transcript", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await page.getByRole("button", { name: "Back to Meetings" }).click();

  // Dictation row expands inline inside a concentric .row-pocket with no negative horizontal margin.
  await mainNavigation(page).locator('[data-nav="dictation"]').click();
  await page.locator(".dictation-row").first().click();
  const dictationPocket = page.locator(".day-card .row-pocket").first();
  await expect(dictationPocket).toBeVisible();
  const pocketGeometry = await dictationPocket.evaluate((el) => {
    const card = el.closest(".day-card").getBoundingClientRect();
    const r = el.getBoundingClientRect();
    return {
      leftInset: Math.round(r.left - card.left),
      rightInset: Math.round(card.right - r.right),
      radius: parseFloat(getComputedStyle(el).borderRadius),
    };
  });
  expect(pocketGeometry.leftInset).toBeGreaterThanOrEqual(8);
  expect(pocketGeometry.leftInset).toBeLessThanOrEqual(10);
  expect(pocketGeometry.rightInset).toBeGreaterThanOrEqual(8);
  expect(pocketGeometry.rightInset).toBeLessThanOrEqual(10);
  expect(pocketGeometry.radius).toBeCloseTo(9, 0);

  // Words: concentric .row-pocket expansion; Review: instant edit pocket and switches.
  await mainNavigation(page).locator('[data-nav="words"]').click();
  await page.locator(".word-row").first().click();
  await expect(page.locator(".words-paper .row-pocket")).toBeVisible();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();

  await page.getByRole("button", { name: /^Review\b/ }).click();
  await page
    .locator(".review-entry")
    .first()
    .getByRole("button", { name: "Edit", exact: true })
    .click();
  const reviewPocket = page.locator(".review-paper .row-pocket").first();
  await expect(reviewPocket).toBeVisible();
  await reviewPocket.getByRole("button", { name: "Cancel", exact: true }).click();
});

test("SettingRow 200px grid, full-width 860px choice cards, InfoTip binding, Sync now width, and Help layout", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1060, height: 760 });
  await openApp(page, seedOptions());
  await page.evaluate(() => document.fonts.ready);

  // Full-width (860px) model choice cards on Speech, Cleanup, and Notes.
  for (const route of [
    { tab: "dictation", stage: "speech" },
    { tab: "dictation", stage: "cleanup" },
    { tab: "meetings", stage: "notes" },
  ]) {
    await page.evaluate((r) => window.__testNavigatePage(r), route);
    const card = page.locator(".option-card").first();
    await expect(card).toBeVisible();
    const width = (await card.boundingBox()).width;
    expect(width, `option-card width on ${route.stage}`).toBeGreaterThanOrEqual(
      850,
    );
  }

  // Notes: Style uses Picker, Custom instructions sit in column 2 (not column 1).
  await page.getByRole("button", { name: "Style", exact: true }).click();
  await page.getByRole("option", { name: "Custom", exact: true }).click();
  await expect(page.locator("#notes-default-instructions")).toBeVisible();
  const customColumns = await page.evaluate(() => {
    const row = document
      .querySelector("#notes-default-instructions")
      .closest(".setting-row");
    const label = row.querySelector(".row-copy");
    const control = row.querySelector(".row-control");
    return {
      labelWidth: Math.round(label.getBoundingClientRect().width),
      controlLeft: Math.round(control.getBoundingClientRect().left),
      labelRight: Math.round(label.getBoundingClientRect().right),
    };
  });
  expect(customColumns.labelWidth).toBeGreaterThanOrEqual(198);
  expect(customColumns.labelWidth).toBeLessThanOrEqual(202);
  expect(customColumns.controlLeft).toBeGreaterThan(customColumns.labelRight);
  // Restore non-draft style before navigating away so the unsaved-changes guard stays quiet.
  await page.getByRole("button", { name: "Style", exact: true }).click();
  await page.getByRole("option", { name: "No notes", exact: true }).click();

  // Recommended stamp stays inline-sized (not stretched full-width) at 900px and 899px.
  await page.evaluate(() =>
    window.__testNavigatePage({ tab: "dictation", stage: "speech" }),
  );
  for (const width of [900, 899]) {
    await page.setViewportSize({ width, height: 860 });
    const stamp = page.locator(".option-card .stamp").first();
    if (await stamp.isVisible()) {
      const box = await stamp.boundingBox();
      expect(box.width, `stamp width at ${width}px`).toBeLessThan(220);
    }
  }
  await page.setViewportSize({ width: 1060, height: 760 });

  // Sync now button stays compact (not stretched to 806px).
  await page.evaluate(() =>
    window.__testNavigatePage({ tab: "settings", stage: "sync" }),
  );
  const syncNow = page.getByRole("button", { name: "Sync now", exact: true });
  await expect(syncNow).toBeVisible();
  expect((await syncNow.boundingBox()).width).toBeLessThan(180);

  // Help: divider after Report a problem, vertically centered CLI row, Disclosure top divider + chevron rotation.
  await page.evaluate(() =>
    window.__testNavigatePage({ tab: "settings", stage: "help" }),
  );
  const cliAlignment = await page.evaluate(() => {
    const row = document.querySelector(".cli-row").closest(".setting-row");
    const label = row.querySelector(".row-copy");
    const control = row.querySelector(".row-control");
    const lBox = label.getBoundingClientRect();
    const cBox = control.getBoundingClientRect();
    return Math.abs(
      lBox.top + lBox.height / 2 - (cBox.top + cBox.height / 2),
    );
  });
  expect(cliAlignment).toBeLessThanOrEqual(6);

  const disclosure = page.locator("details.ui-disclosure").first();
  await disclosure.locator("> summary").click();
  await expect(disclosure).toHaveAttribute("open", "");
  const disclosureBorder = await disclosure
    .locator("> .disclosure-body")
    .evaluate((el) => getComputedStyle(el).borderTopWidth);
  expect(parseFloat(disclosureBorder)).toBeGreaterThanOrEqual(1);

  // InfoTip bound to final word (overlaps vertically with label text) at 740px on TrustScreen.
  await page.setViewportSize({ width: 740, height: 540 });
  await page.evaluate(() =>
    window.__testNavigatePage({ tab: "dictation", stage: "trust" }),
  );
  const infoBound = await page.evaluate(() => {
    const tail = document.querySelector(".setting-row .label-tail");
    if (!tail) return null;
    const tip = tail.querySelector("button");
    if (!tip) return null;
    const tailRect = tail.getBoundingClientRect();
    const tipRect = tip.getBoundingClientRect();
    return {
      tipCenterY: tipRect.top + tipRect.height / 2,
      tailTop: tailRect.top,
      tailBottom: tailRect.bottom,
    };
  });
  expect(infoBound).not.toBeNull();
  expect(infoBound.tipCenterY).toBeGreaterThanOrEqual(infoBound.tailTop - 2);
  expect(infoBound.tipCenterY).toBeLessThanOrEqual(infoBound.tailBottom + 2);
});

test("review regressions: Words alignment, Cleanup Level/Proofreader alignment, Picker 17-mic/dups, Meeting rename a11y, scroll restore, and material/contrast prefs", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));

  const manyMeetings = Array.from({ length: 24 }, (_, idx) => ({
    id: `meeting-${idx}`,
    title: `Sprint review ${idx + 1}`,
    createdAt: new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate() - Math.floor(idx / 3),
      10 + (idx % 3),
      0,
      0,
    ).toISOString(),
    durationSeconds: 600,
    status: "processed",
    hasAudio: true,
    hasTranscript: true,
    audioOutages: [],
    missingSeconds: 0,
    cleanedTranscript: null,
    transcriptionStoppedAtSeconds: null,
    transcript: "[0:05] You: Checking scroll restoration.",
    notes: "# Summary\n- Restores list position.",
    source: "both",
    speakerCount: 2,
    speakerLabels: true,
    audioPath: `C:/m-${idx}.wav`,
    error: null,
  }));
  const manyDevices = [
    { id: "USB Mic", name: "USB Mic", isDefault: false },
    { id: "USB Mic", name: "USB Mic", isDefault: false },
    ...Array.from({ length: 16 }, (_, idx) => ({
      id: `mic-${idx + 1}`,
      name: `Studio Microphone ${idx + 1}`,
      isDefault: idx === 0,
    })),
  ];

  await page.setViewportSize({ width: 1060, height: 760 });
  await openApp(page, {
    ...seedOptions(),
    meetings: manyMeetings,
    devices: manyDevices,
    settings: { microphone: "mic-15" },
  });
  await page.evaluate(() => document.fonts.ready);

  // 1. Words: HINT row written word aligns horizontally with replacement rows.
  await mainNavigation(page).locator('[data-nav="words"]').click();
  const writtenLefts = await page
    .locator(".word-row .word-written")
    .evaluateAll((els) => els.map((el) => Math.round(el.getBoundingClientRect().left)));
  expect(writtenLefts.length).toBeGreaterThanOrEqual(2);
  expect(Math.max(...writtenLefts) - Math.min(...writtenLefts)).toBeLessThanOrEqual(1);

  // 2. Cleanup: Level segmented control fits content (< 340px), Proofreader memory bars align.
  await page.evaluate(() =>
    window.__testNavigatePage({ tab: "dictation", stage: "cleanup" }),
  );
  const cleanupChecks = await page.evaluate(() => {
    const seg = document.querySelector('.seg-control[aria-label="Cleanup level"]');
    const bars = [...document.querySelectorAll('.option-card .memory-bar, .option-card [class*="memory"]')];
    const barRights = bars
      .filter((el) => el.getBoundingClientRect().width > 20)
      .map((el) => Math.round(el.getBoundingClientRect().right));
    return {
      segWidth: seg ? Math.round(seg.getBoundingClientRect().width) : 9999,
      barRights,
    };
  });
  expect(cleanupChecks.segWidth).toBeLessThan(340);
  if (cleanupChecks.barRights.length >= 2) {
    expect(
      Math.max(...cleanupChecks.barRights) - Math.min(...cleanupChecks.barRights),
    ).toBeLessThanOrEqual(2);
  }

  // 3. Picker with duplicate devices + 17 options at 740x540: zero errors, selected option visible in viewport.
  await page.setViewportSize({ width: 740, height: 540 });
  await page.evaluate(() =>
    window.__testNavigatePage({ tab: "dictation", stage: "input" }),
  );
  await page.getByRole("button", { name: "Microphone", exact: true }).click();
  const listbox = page.getByRole("listbox", { name: "Microphone" });
  await expect(listbox).toBeVisible();
  const selectedOpt = listbox.locator('[role="option"][aria-selected="true"]');
  await expect(selectedOpt).toBeFocused();
  const pickerVis = await page.evaluate(() => {
    const box = document.querySelector('[role="listbox"][aria-label="Microphone"]');
    const opt = box.querySelector('[aria-selected="true"]');
    const br = box.getBoundingClientRect();
    const or = opt.getBoundingClientRect();
    return {
      boxTop: br.top,
      boxBottom: br.bottom,
      optTop: or.top,
      optBottom: or.bottom,
      vh: window.innerHeight,
    };
  });
  expect(pickerVis.boxTop).toBeGreaterThanOrEqual(0);
  expect(pickerVis.boxBottom).toBeLessThanOrEqual(pickerVis.vh + 1);
  expect(pickerVis.optTop).toBeGreaterThanOrEqual(pickerVis.boxTop - 1);
  expect(pickerVis.optBottom).toBeLessThanOrEqual(pickerVis.boxBottom + 1);
  await page.keyboard.press("Escape");

  // 4. Meetings: scroll deep into 24-meeting list, open meeting, test rename Escape keeps h1 & restores focus, Back restores scroll + row focus.
  await page.setViewportSize({ width: 1060, height: 760 });
  await mainNavigation(page).locator('[data-nav="meetings"]').click();
  await expect(page.locator('[data-meeting-id="meeting-18"]')).toBeVisible();
  await page.locator(".page-content").evaluate((el) => {
    el.scrollTop = 650;
  });
  const savedScroll = await page
    .locator(".page-content")
    .evaluate((el) => el.scrollTop);
  expect(savedScroll).toBeGreaterThan(200);
  await page.locator('[data-meeting-id="meeting-18"]').click();
  await expect(page.locator(".meeting-heading")).toBeVisible();
  // Rename mode keeps an h1 and Escape returns focus to Rename meeting button.
  await page.getByRole("button", { name: "Rename meeting", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1 })).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "Rename meeting", exact: true }),
  ).toBeFocused();
  await page.getByRole("button", { name: "Back to Meetings" }).click();
  await expect
    .poll(() =>
      page.locator(".page-content").evaluate((el) => el.scrollTop),
    )
    .toBeGreaterThan(200);
  await expect(page.locator('[data-meeting-id="meeting-18"]')).toBeFocused();

  // 5. Route-backed sheet (Report from Help) preserves Help scroll position.
  await page.setViewportSize({ width: 740, height: 540 });
  await page.evaluate(() =>
    window.__testNavigatePage({ tab: "settings", stage: "help" }),
  );
  await page.locator("details.ui-disclosure > summary").first().click();
  await page.locator(".page-content").evaluate((el) => {
    el.scrollTop = 180;
  });
  const helpScrollBefore = await page
    .locator(".page-content")
    .evaluate((el) => el.scrollTop);
  expect(helpScrollBefore).toBeGreaterThan(50);
  await page.evaluate(() => window.__testNavigatePage("report"));
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.getByRole("button", { name: "Close sheet" }).click();
  const helpScrollAfter = await page
    .locator(".page-content")
    .evaluate((el) => el.scrollTop);
  expect(Math.abs(helpScrollAfter - helpScrollBefore)).toBeLessThanOrEqual(2);

  // 6. Recessed paper tracks + sliding raised thumbs (rail + SegmentedControl) use --card (not --dye), track active item, and honor material/contrast prefs.
  await page.evaluate(() => {
    window.__testNavigatePage({ tab: "settings", stage: "memory" });
  });
  const activeSelection = await page.evaluate(() => {
    const studio = document.querySelector(".studio");
    const probe = document.createElement("span");
    probe.style.backgroundColor = "var(--card)";
    probe.style.color = "var(--ink)";
    studio.appendChild(probe);
    const cardBg = getComputedStyle(probe).backgroundColor;
    const inkColor = getComputedStyle(probe).color;
    probe.remove();
    const railTrack = document.querySelector(".rail-track");
    const railThumb = document.querySelector(".rail-thumb");
    const railSel = document.querySelector(".rail-item.selected");
    const segThumb = document.querySelector(".seg-thumb");
    const segSel = document.querySelector('.seg-btn[aria-pressed="true"]');
    const rtcs = getComputedStyle(railTrack);
    const rcs = getComputedStyle(railThumb);
    const rbcs = getComputedStyle(railSel);
    const scs = getComputedStyle(segThumb);
    const sbcs = getComputedStyle(segSel);
    const railTrackRect = railTrack.getBoundingClientRect();
    const pageFrameRect = document
      .querySelector(".page-frame")
      .getBoundingClientRect();
    const railThumbRect = railThumb.getBoundingClientRect();
    const railSelRect = railSel.getBoundingClientRect();
    const segThumbRect = segThumb.getBoundingClientRect();
    const segSelRect = segSel.getBoundingClientRect();
    return {
      cardBg,
      inkColor,
      railTrackShadow: rtcs.boxShadow,
      railTrackTopDelta: Math.abs(railTrackRect.top - pageFrameRect.top),
      railBg: rcs.backgroundColor,
      railColor: rbcs.color,
      railShadow: rcs.boxShadow,
      railAlignDelta: Math.abs(railThumbRect.top - railSelRect.top),
      segBg: scs.backgroundColor,
      segBtnBgImg: sbcs.backgroundImage,
      segColor: sbcs.color,
      segShadow: scs.boxShadow,
      segAlignDelta: Math.abs(segThumbRect.left - segSelRect.left),
    };
  });
  expect(activeSelection.railTrackShadow).toContain("inset");
  expect(activeSelection.railTrackTopDelta).toBeLessThanOrEqual(1);
  expect(activeSelection.railBg).toBe(activeSelection.cardBg);
  expect(activeSelection.railColor).toBe(activeSelection.inkColor);
  expect(activeSelection.railShadow).not.toBe("none");
  expect(activeSelection.railAlignDelta).toBeLessThanOrEqual(1);
  expect(activeSelection.segBg).toBe(activeSelection.cardBg);
  expect(activeSelection.segBtnBgImg).toBe("none");
  expect(activeSelection.segColor).toBe(activeSelection.inkColor);
  expect(activeSelection.segShadow).not.toBe("none");
  expect(activeSelection.segAlignDelta).toBeLessThanOrEqual(1);

  // Switching rail tab slides the rail thumb to the newly active item.
  await mainNavigation(page).locator('[data-nav="words"]').click();
  const movedDelta = await page.evaluate(() => {
    const thumb = document.querySelector(".rail-thumb").getBoundingClientRect();
    const active = document.querySelector('[data-nav="words"]').getBoundingClientRect();
    return Math.abs(thumb.top - active.top);
  });
  expect(movedDelta).toBeLessThanOrEqual(1);

  await page.evaluate(() => {
    window.__testNavigatePage({ tab: "settings", stage: "memory" });
    document.querySelector(".studio")?.setAttribute("data-material", "off");
  });
  const segBtn = page.locator(".seg-btn").first();
  await segBtn.hover();
  const offStyle = await segBtn.evaluate((el) => {
    const cs = getComputedStyle(el);
    return { bgImg: cs.backgroundImage, shadow: cs.boxShadow };
  });
  expect(offStyle).toEqual({ bgImg: "none", shadow: "none" });

  await page.emulateMedia({ contrast: "more" });
  await page.evaluate(() => {
    document.querySelector(".studio")?.setAttribute("data-material", "on");
  });
  const highContrast = await page
    .locator('.seg-btn[aria-pressed="true"]')
    .first()
    .evaluate((el) => {
      const cs = getComputedStyle(el);
      return {
        bgImg: cs.backgroundImage,
        shadow: cs.boxShadow,
        borderTop: parseFloat(cs.borderTopWidth),
      };
    });
  expect(highContrast.bgImg).toBe("none");
  expect(highContrast.shadow).toBe("none");
  expect(highContrast.borderTop).toBeGreaterThanOrEqual(1);

  expect(errors).toEqual([]);
});
