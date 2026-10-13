import assert from "node:assert/strict";
import test from "node:test";
import {
  meetingSourceWarning,
  meetingStartPrompt,
  prepareMeetingStart,
} from "./meeting-source";
import copy from "./meeting-source-copy.json";
import { previewSnapshot, settingsSchema } from "./protocol";

test("each one-sided source warns about the missing side", () => {
  assert.equal(meetingSourceWarning("both"), null);
  assert.equal(
    meetingSourceWarning("microphone"),
    "Only your microphone will be recorded. On a Zoom, Teams or Meet call, the other people won't be in the recording.",
  );
  assert.equal(
    meetingSourceWarning("system"),
    "Only sound from this computer will be recorded. Your own voice won't be in the recording.",
  );
});

test("first meeting combines consent and any source warning in one prompt", () => {
  for (const meetingSource of ["both", "microphone", "system"] as const) {
    const prompt = meetingStartPrompt({
      meetingSource,
      meetingConsentAcknowledged: false,
    });
    assert.ok(prompt);
    assert.equal(prompt.cancelLabel, "Cancel");
    assert.equal(prompt.detail.split(copy.reminder).length, 2);
    if (meetingSource === "both") {
      assert.equal(prompt.detail, copy.reminder);
      assert.equal(prompt.label, "Got it");
      assert.equal(prompt.alternativeLabel, undefined);
    } else {
      const warning = meetingSourceWarning(meetingSource);
      assert.ok(warning);
      assert.ok(prompt.detail.includes(warning));
      assert.equal(prompt.label, "Got it, record both sides");
      assert.equal(prompt.alternativeLabel, "Got it, start anyway");
    }
  }
});

test("acknowledged meetings retain only the one-sided warning", () => {
  assert.equal(
    meetingStartPrompt({
      meetingSource: "both",
      meetingConsentAcknowledged: true,
    }),
    null,
  );
  for (const meetingSource of ["microphone", "system"] as const) {
    const prompt = meetingStartPrompt({
      meetingSource,
      meetingConsentAcknowledged: true,
    });
    assert.equal(prompt?.detail, meetingSourceWarning(meetingSource));
    assert.equal(prompt?.label, "Record both sides");
    assert.equal(prompt?.alternativeLabel, "Start anyway");
  }
});

test("Cancel never acknowledges or starts, and the next attempt still asks", async () => {
  for (const meetingSource of ["both", "microphone", "system"] as const) {
    const settings = { meetingSource, meetingConsentAcknowledged: false };
    let prompts = 0;
    for (let attempt = 0; attempt < 2; attempt++) {
      assert.equal(
        await prepareMeetingStart(
          settings,
          async () => {
            prompts++;
            return false;
          },
          async () => assert.fail("Cancel must not save settings"),
        ),
        null,
      );
    }
    assert.equal(prompts, 2);
    assert.equal(settings.meetingConsentAcknowledged, false);
  }
});

test("Got it waits for a persisted acknowledgement before allowing capture", async () => {
  const calls: string[] = [];
  let release: (() => void) | undefined;
  const start = prepareMeetingStart(
    { meetingSource: "both", meetingConsentAcknowledged: false },
    async () => true,
    async (patch) => {
      assert.deepEqual(patch, { meetingConsentAcknowledged: true });
      calls.push("saving");
      await new Promise<void>((resolve) => {
        release = resolve;
      });
      calls.push("saved");
    },
  ).then((source) => {
    calls.push(`start ${source}`);
  });
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, ["saving"]);
  release?.();
  await start;
  assert.deepEqual(calls, ["saving", "saved", "start both"]);
});

test("combined decisions save consent and the chosen source together", async () => {
  for (const meetingSource of ["microphone", "system"] as const) {
    for (const choice of [true, "alternative"] as const) {
      let saves = 0;
      let prompts = 0;
      const source = await prepareMeetingStart(
        { meetingSource, meetingConsentAcknowledged: false },
        async () => {
          prompts++;
          return choice;
        },
        async (patch) => {
          saves++;
          assert.deepEqual(
            patch,
            choice === true
              ? { meetingConsentAcknowledged: true, meetingSource: "both" }
              : { meetingConsentAcknowledged: true },
          );
        },
      );
      assert.equal(prompts, 1);
      assert.equal(saves, 1);
      assert.equal(source, choice === true ? "both" : meetingSource);
    }
  }
});

test("failed acknowledgement persistence prevents a start", async () => {
  await assert.rejects(
    prepareMeetingStart(
      { meetingSource: "both", meetingConsentAcknowledged: false },
      async () => true,
      async () => {
        throw new Error("disk unavailable");
      },
    ),
    /disk unavailable/,
  );
});

test("a dismissed page or intervening capture invalidates a pending start", async () => {
  for (const invalidateAt of ["prompt", "save"] as const) {
    let current = true;
    let saves = 0;
    assert.equal(
      await prepareMeetingStart(
        { meetingSource: "both", meetingConsentAcknowledged: false },
        async () => {
          if (invalidateAt === "prompt") current = false;
          return true;
        },
        async () => {
          saves++;
          current = false;
        },
        () => current,
      ),
      null,
    );
    assert.equal(saves, invalidateAt === "prompt" ? 0 : 1);
  }
});

test("later both-source starts neither prompt nor save", async () => {
  assert.equal(
    await prepareMeetingStart(
      { meetingSource: "both", meetingConsentAcknowledged: true },
      async () =>
        assert.fail("Acknowledged two-source meeting must not prompt"),
      async () => assert.fail("Acknowledged two-source meeting needs no save"),
    ),
    "both",
  );
});

test("legacy settings default to an unacknowledged reminder", () => {
  const { meetingConsentAcknowledged: _acknowledged, ...legacy } =
    previewSnapshot.settings;
  assert.equal(settingsSchema.parse(legacy).meetingConsentAcknowledged, false);
});

test("missing source and the preview use both; saved deliberate sources round-trip", () => {
  const {
    meetingSource: _source,
    meetingSourceMigrated: _migrated,
    ...legacy
  } = previewSnapshot.settings;
  assert.equal(settingsSchema.parse(legacy).meetingSource, "both");
  assert.equal(settingsSchema.parse(legacy).meetingSourceMigrated, false);
  assert.equal(previewSnapshot.settings.meetingSource, "both");
  for (const source of ["microphone", "system", "both"] as const) {
    const settings = settingsSchema.parse({
      ...legacy,
      meetingSource: source,
      meetingSourceMigrated: true,
    });
    assert.equal(settings.meetingSource, source);
    assert.equal(settings.meetingSourceMigrated, true);
  }
});
