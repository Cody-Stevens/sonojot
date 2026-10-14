import assert from "node:assert/strict";
import test from "node:test";
import {
  meetingSourceWarning,
  meetingStartPrompt,
  prepareMeetingStart,
  recordingSystemNotice,
  systemAudioFixable,
  systemAudioState,
  tabMeetingWarning,
  type MeetingSource,
} from "./meeting-source";
import copy from "./meeting-source-copy.json";
import {
  previewSnapshot,
  settingsSchema,
  snapshotSchema,
  type SystemAudioStatus,
} from "./protocol";

const statuses: SystemAudioStatus[] = [
  "ready",
  "notDetermined",
  "denied",
  "unsupported",
];
const sources: MeetingSource[] = ["both", "microphone", "system"];
const acknowledged = (meetingSource: MeetingSource) => ({
  meetingSource,
  meetingConsentAcknowledged: true,
});

test("each one-sided source warns about the missing side", () => {
  assert.equal(meetingSourceWarning("both"), null);
  assert.equal(
    meetingSourceWarning("microphone"),
    "Only your microphone will be recorded. Other people on a Zoom, Teams or Meet call won't be in the recording.",
  );
  assert.equal(
    meetingSourceWarning("system"),
    "Only sound from this computer will be recorded. Your own voice won't be in the recording.",
  );
});

test("only a missing permission is fixable", () => {
  assert.deepEqual(
    statuses.map(systemAudioFixable),
    [false, true, true, false],
  );
});

test("both sides with working system audio start without a dialog", () => {
  assert.equal(meetingStartPrompt(acknowledged("both"), "ready"), null);
});

test("every start that would miss the other people needs an explicit choice", () => {
  for (const source of sources)
    for (const status of statuses) {
      const prompt = meetingStartPrompt(acknowledged(source), status);
      if (source === "both" && status === "ready") continue;
      assert.ok(prompt, `${source}/${status} must ask`);
      assert.equal(prompt.cancelLabel, "Cancel");
      // A missing other side is always named; a system-only source names your voice.
      if (source !== "system" || status === "unsupported")
        assert.ok(prompt.detail.includes(copy.microphone), `${source}/${status}`);
    }
});

test("microphone only offers both sides, or the fix when permission is missing", () => {
  const ready = meetingStartPrompt(acknowledged("microphone"), "ready");
  assert.deepEqual(ready, {
    title: "Record only your microphone?",
    detail: copy.microphone,
    label: "Record both sides",
    primary: "both",
    alternativeLabel: "Record microphone only",
    alternative: "start",
    cancelLabel: "Cancel",
  });
  for (const status of ["notDetermined", "denied"] as const) {
    const prompt = meetingStartPrompt(acknowledged("microphone"), status);
    assert.equal(prompt?.label, "Fix system audio");
    assert.equal(prompt?.primary, "fix");
    assert.equal(prompt?.alternativeLabel, "Record microphone only");
    assert.equal(prompt?.alternative, "start");
  }
  const unsupported = meetingStartPrompt(acknowledged("microphone"), "unsupported");
  assert.equal(unsupported?.label, "Record microphone only");
  assert.equal(unsupported?.alternativeLabel, undefined);
  assert.ok(unsupported?.detail.includes(copy.systemUnsupported));
});

test("both sides without permission says system audio is off and offers the fix", () => {
  for (const status of ["notDetermined", "denied"] as const) {
    assert.deepEqual(meetingStartPrompt(acknowledged("both"), status), {
      title: "System audio is off",
      detail: `${copy.microphone}\n\n${copy.systemPermission}`,
      label: "Fix system audio",
      primary: "fix",
      alternativeLabel: "Record microphone only",
      alternative: "start",
      cancelLabel: "Cancel",
    });
  }
  const unsupported = meetingStartPrompt(acknowledged("both"), "unsupported");
  assert.equal(unsupported?.primary, "microphone");
  assert.equal(unsupported?.alternative, undefined);
});

test("system only without permission can record the microphone until it is allowed", () => {
  const prompt = meetingStartPrompt(acknowledged("system"), "denied");
  assert.equal(prompt?.detail, copy.systemOnlyBlocked);
  assert.equal(prompt?.primary, "fix");
  assert.equal(prompt?.alternative, "both");
  assert.equal(meetingStartPrompt(acknowledged("system"), "ready")?.alternativeLabel, "Record system audio only");
  assert.equal(meetingStartPrompt(acknowledged("system"), "unsupported")?.primary, "microphone");
});

test("first meeting combines consent and any source warning in one prompt", () => {
  for (const source of sources)
    for (const status of statuses) {
      const prompt = meetingStartPrompt(
        { meetingSource: source, meetingConsentAcknowledged: false },
        status,
      );
      assert.ok(prompt);
      assert.equal(prompt.title, copy.title);
      assert.equal(prompt.detail.split(copy.reminder).length, 2);
      assert.ok(prompt.detail.startsWith(copy.reminder));
      const plain = meetingStartPrompt(acknowledged(source), status);
      if (plain) {
        assert.equal(prompt.detail, `${copy.reminder}\n\n${plain.detail}`);
        assert.equal(prompt.label, plain.label);
      } else {
        assert.equal(prompt.detail, copy.reminder);
        assert.equal(prompt.label, "Got it");
      }
    }
});

test("Fix system audio never acknowledges, saves or starts", async () => {
  for (const source of sources) {
    const plan = await prepareMeetingStart(
      { meetingSource: source, meetingConsentAcknowledged: false },
      "denied",
      async () => true,
      async () => assert.fail("Fix must not save settings"),
    );
    assert.deepEqual(plan, { action: "fix" });
  }
});

test("Record microphone only keeps a fixable system side so it can join later", async () => {
  const saves: object[] = [];
  const plan = await prepareMeetingStart(
    acknowledged("both"),
    "notDetermined",
    async () => "alternative",
    async (patch) => {
      saves.push(patch);
    },
  );
  assert.deepEqual(plan, { action: "start", source: "both" });
  assert.equal(saves.length, 0);
  const later: object[] = [];
  assert.deepEqual(
    await prepareMeetingStart(
      acknowledged("system"),
      "denied",
      async () => "alternative",
      async (patch) => {
        later.push(patch);
      },
    ),
    { action: "start", source: "both" },
  );
  assert.deepEqual(later, [{ meetingSource: "both" }]);
});

test("an OS without system audio saves microphone only before starting", async () => {
  const saves: object[] = [];
  assert.deepEqual(
    await prepareMeetingStart(
      acknowledged("both"),
      "unsupported",
      async () => true,
      async (patch) => {
        saves.push(patch);
      },
    ),
    { action: "start", source: "microphone" },
  );
  assert.deepEqual(saves, [{ meetingSource: "microphone" }]);
});

test("Cancel never acknowledges or starts, and the next attempt still asks", async () => {
  for (const meetingSource of sources)
    for (const status of statuses) {
      const settings = { meetingSource, meetingConsentAcknowledged: false };
      let prompts = 0;
      for (let attempt = 0; attempt < 2; attempt++) {
        assert.equal(
          await prepareMeetingStart(
            settings,
            status,
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
    }
});

test("Got it waits for a persisted acknowledgement before allowing capture", async () => {
  const calls: string[] = [];
  let release: (() => void) | undefined;
  const start = prepareMeetingStart(
    { meetingSource: "both", meetingConsentAcknowledged: false },
    "ready",
    async () => true,
    async (patch) => {
      assert.deepEqual(patch, { meetingConsentAcknowledged: true });
      calls.push("saving");
      await new Promise<void>((resolve) => {
        release = resolve;
      });
      calls.push("saved");
    },
  ).then((plan) => {
    calls.push(`start ${plan?.action === "start" ? plan.source : "none"}`);
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
      const plan = await prepareMeetingStart(
        { meetingSource, meetingConsentAcknowledged: false },
        "ready",
        async () => choice,
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
      assert.equal(saves, 1);
      assert.deepEqual(plan, {
        action: "start",
        source: choice === true ? "both" : meetingSource,
      });
    }
  }
});

test("failed persistence prevents a start", async () => {
  await assert.rejects(
    prepareMeetingStart(
      { meetingSource: "both", meetingConsentAcknowledged: false },
      "ready",
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
        "ready",
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

test("the screen tab never holds a both-sides quick start", () => {
  for (const status of statuses) assert.equal(tabMeetingWarning("both", status), null);
});

test("the screen tab offers the fix instead of an option that cannot work", () => {
  assert.deepEqual(tabMeetingWarning("microphone", "ready"), {
    text: copy.microphone,
    start: { label: "Start anyway", source: "microphone" },
    primary: { label: "Record both sides", action: "both" },
  });
  assert.deepEqual(tabMeetingWarning("microphone", "denied")?.primary, {
    label: "Fix system audio",
    action: "fix",
  });
  assert.equal(tabMeetingWarning("microphone", "unsupported")?.primary, null);
  assert.deepEqual(tabMeetingWarning("system", "notDetermined"), {
    text: copy.systemOnlyBlocked,
    start: { label: "Record microphone only", source: "both" },
    primary: { label: "Fix system audio", action: "fix" },
  });
  assert.deepEqual(tabMeetingWarning("system", "unsupported")?.start, {
    label: "Record microphone only",
    source: "microphone",
  });
});

test("live notices name a missing other side; a device loss uses the source status", () => {
  assert.deepEqual(recordingSystemNotice("permission"), {
    text: copy.recordingPermission,
    fix: "Allow system audio",
  });
  assert.equal(recordingSystemNotice("unsupported")?.fix, null);
  assert.equal(recordingSystemNotice("lost"), null);
  assert.equal(recordingSystemNotice(null), null);
});

test("permission states are distinct and only fixable ones offer an action", () => {
  const states = statuses.map((status) => systemAudioState(status, "macos"));
  assert.deepEqual(
    states.map((state) => state.label),
    ["Ready", "Not allowed yet", "Not allowed", "Unavailable"],
  );
  assert.deepEqual(
    states.map((state) => state.action),
    [null, "Allow system audio", "Open System Settings", null],
  );
  assert.match(systemAudioState("ready", "windows").detail, /needs no permission/);
});

test("legacy snapshots default to an unsupported, never ready, system audio status", () => {
  const { systemAudioStatus: _status, ...capabilities } =
    previewSnapshot.capabilities;
  const parsed = snapshotSchema.parse({ ...previewSnapshot, capabilities });
  assert.equal(parsed.capabilities.systemAudioStatus, "unsupported");
  assert.equal(parsed.meetingSystemAudio, null);
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
    systemAudioDefaultApplied: _applied,
    ...legacy
  } = previewSnapshot.settings;
  assert.equal(settingsSchema.parse(legacy).meetingSource, "both");
  assert.equal(settingsSchema.parse(legacy).meetingSourceMigrated, false);
  assert.equal(settingsSchema.parse(legacy).systemAudioDefaultApplied, false);
  for (const source of sources) {
    const settings = settingsSchema.parse({
      ...legacy,
      meetingSource: source,
      meetingSourceMigrated: true,
    });
    assert.equal(settings.meetingSource, source);
  }
});
