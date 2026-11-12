import { app } from "./app.svelte";
import { call, chooseDictationExport } from "./api";
import type { DictationRecord } from "./protocol";
import type { DictationVersion } from "./dictation-history";

export async function copyDictation(
  record: DictationRecord,
  version: DictationVersion = "published",
) {
  await app.perform(
    "history-copy",
    () =>
      version === "pre_model"
        ? call("copy_history_pre_model", { id: record.id })
        : call("copy_history", { id: record.id, original: version === "raw" }),
    "Text copied.",
  );
}

export async function exportDictation(record: DictationRecord) {
  await app.perform("history-export", async () => {
    const path = await chooseDictationExport(
      `Dictation-${record.createdAt.slice(0, 19).replaceAll(":", "-")}.md`,
    );
    if (!path) return;
    await call("export_dictation", { id: record.id, path });
    app.notice = "Dictation exported.";
  });
}

export async function deleteDictation(
  record: DictationRecord,
): Promise<boolean> {
  let note = " Derived learning without another source is removed too.";
  try {
    const dependents = (await call("get_learning")).filter((entry) =>
      entry.sourceIds.includes(record.id),
    );
    const scopes = [
      ...new Set(
        dependents.flatMap((entry) => (entry.scope ? [entry.scope] : [])),
      ),
    ];
    if (dependents.length)
      note = ` ${dependents.length} learned ${dependents.length === 1 ? "entry loses" : "entries lose"} this source${scopes.length ? ` (${scopes.join(", ")})` : ""}; entries without another source are removed.`;
  } catch {
    /* Keep the general learning warning if its inventory is unavailable. */
  }
  if (
    !(await app.confirm(
      "Delete this dictation?",
      `This removes the saved original, finished text and kept audio.${note} Words you added stay.`,
      "Delete",
      true,
    ))
  )
    return false;
  const next = await app.perform("history-delete", () =>
    call("delete_dictation", { id: record.id, confirmed: true }),
  );
  if (!next) return false;
  app.apply(next);
  app.notice = "Dictation deleted.";
  return true;
}
