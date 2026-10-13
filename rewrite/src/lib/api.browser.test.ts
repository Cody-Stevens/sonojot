import { strict as assert } from "node:assert";
import { test } from "node:test";
import { call, native, chooseMedia, chooseExport } from "./api";

test("browser preview never reports a native mutation or file dialog as successful", async () => {
  assert.equal(native, false);
  await assert.rejects(call("start_dictation"), /desktop app/);
  await assert.rejects(call("save_vocabulary", { entries: [] }), /desktop app/);
  await assert.rejects(chooseMedia(), /desktop app/);
  await assert.rejects(chooseExport("meeting.md", "md"), /desktop app/);
});
