import { onMount, untrack } from "svelte";
import { z } from "zod";
import { listen } from "@tauri-apps/api/event";
import { sync, syncAction, refreshSync } from "./sync.svelte";
import { app } from "./app.svelte";

const schema = z.object({
  status: z.string(),
  direction: z.enum(["upload", "download"]).nullable().optional(),
  audioOverride: z.boolean().nullable(),
  audioEnabled: z.boolean(),
  audioAvailable: z.boolean(),
  deleteConflict: z.boolean(),
  conflicts: z.array(
    z.object({
      sid: z.string(),
      view: z.string(),
      version: z.string(),
      text: z.string(),
    }),
  ),
  fieldConflicts: z.record(z.string(), z.unknown()).nullable(),
});
export type MeetingSyncState = z.infer<typeof schema>;

/** What the cloud beside the meeting title shows. */
export type SyncGlyph = {
  icon: "cloud-check" | "cloud-up" | "cloud-down" | "cloud-alert" | "cloud-off";
  label: string;
  attention: boolean;
};

export function syncGlyph(
  status: MeetingSyncState | null,
  downloading: boolean,
): SyncGlyph | null {
  if (downloading)
    return { icon: "cloud-down", label: "Downloading", attention: false };
  if (!status) return null;
  const conflicted =
    status.conflicts.length > 0 ||
    status.deleteConflict ||
    Object.keys(status.fieldConflicts ?? {}).length > 0;
  if (conflicted)
    return { icon: "cloud-alert", label: "Needs a choice", attention: true };
  switch (status.status) {
    case "synced":
      return { icon: "cloud-check", label: "Synced", attention: false };
    case "storageFull":
      return {
        icon: "cloud-alert",
        label: "Not synced: storage full",
        attention: true,
      };
    case "deletedElsewhere":
      return {
        icon: "cloud-alert",
        label: "Sync copy unavailable",
        attention: true,
      };
    case "localOnly":
      return {
        icon: "cloud-off",
        label: "Kept on this computer",
        attention: false,
      };
    default:
      return status.direction === "download"
        ? { icon: "cloud-down", label: "Downloading", attention: false }
        : { icon: "cloud-up", label: "Uploading", attention: false };
  }
}

/**
 * Loads one meeting's sync state and keeps it current. Call during component
 * setup; `id` is read reactively.
 */
export type MeetingSyncStatus = ReturnType<typeof meetingSyncStatus>;

export function meetingSyncStatus(
  id: () => string,
  onchanged: () => Promise<void>,
) {
  let status = $state<MeetingSyncState | null>(null);
  let error = $state("");
  let busy = $state<string | null>(null);
  let request = 0;
  async function reload() {
    const meeting = id();
    if (!meeting) return;
    const current = ++request;
    try {
      const result = schema.parse(
        await syncAction("meeting_status", { id: meeting }),
      );
      if (current === request && meeting === id()) status = result;
    } catch (e) {
      if (current === request) error = String(e);
    }
  }
  $effect(() => {
    id();
    void app.meetingRevision;
    void sync.status?.lastSync;
    untrack(() => {
      status = null;
      error = "";
      if (sync.status?.unlocked) void reload();
    });
  });
  onMount(() => {
    void refreshSync();
    let active = true;
    let dispose: (() => void) | undefined;
    void listen("sync-changed", () => {
      void refreshSync().then(() => reload());
    }).then((fn) => {
      if (active) dispose = fn;
      else fn();
    });
    return () => {
      active = false;
      request++;
      dispose?.();
    };
  });
  async function act(operation: string, args: Record<string, unknown> = {}) {
    busy = operation;
    error = "";
    try {
      await syncAction(operation, { id: id(), ...args });
      if (operation !== "meeting_prioritize") await onchanged();
      await reload();
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }
  return {
    get status() {
      return status;
    },
    get error() {
      return error;
    },
    get busy() {
      return busy;
    },
    reload,
    act,
  };
}
