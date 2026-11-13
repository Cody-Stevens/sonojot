import { invoke, isTauri } from "@tauri-apps/api/core";
import { z } from "zod";
import { app } from "./app.svelte";
const meetingOutcomeSchema = z.object({
  errors: z.number(),
  retryAt: z.number(),
  rateLimited: z.boolean(),
  missing: z.number(),
});
const statusSchema = z.object({
  meetings: meetingOutcomeSchema.optional(),
  signedIn: z.boolean(),
  unlocked: z.boolean(),
  initialized: z.boolean().nullable().optional(),
  server: z.string(),
  email: z.string().optional(),
  device: z.string().optional(),
  recoveryPending: z.boolean().optional(),
  lastSync: z.number().optional(),
  conflicts: z.number().optional(),
  notice: z.string().nullable().optional(),
  pendingReset: z.number().nullable().optional(),
  resetCancelled: z.boolean().optional(),
  error: z.string().nullable().optional(),
});
export type SyncStatus = z.infer<typeof statusSchema>;
export const sync = $state<{
  status: SyncStatus | null;
  busy: boolean;
  error: string;
  meetingResult: z.infer<typeof meetingOutcomeSchema> | null;
}>({ status: null, busy: false, error: "", meetingResult: null });
let pendingOperations = 0;
export async function syncAction(
  operation: string,
  args: Record<string, unknown> = {},
): Promise<unknown> {
  if (!app.snapshot.capabilities.sync)
    throw new Error("Sync requires a prerelease or Sync Test build.");
  if (!isTauri()) throw new Error("Open SonoJot to connect your account.");
  return invoke<unknown>("sync_command", { operation, args });
}
export async function refreshSync(): Promise<void> {
  if (!isTauri() || !app.snapshot.capabilities.sync) return;
  try {
    sync.status = statusSchema.parse(await syncAction("status"));
    sync.error = sync.status.error ?? "";
  } catch (error) {
    sync.error = String(error);
  }
}
export async function runSync(
  operation: string,
  args: Record<string, unknown> = {},
): Promise<unknown> {
  pendingOperations += 1;
  sync.busy = true;
  sync.error = "";
  try {
    const result = await syncAction(operation, args);
    await refreshSync();
    if (operation === "now") {
      const parsed = z
        .object({ meetings: meetingOutcomeSchema })
        .safeParse(result);
      sync.meetingResult = parsed.success ? parsed.data.meetings : null;
    }
    return result;
  } catch (error) {
    sync.error = String(error);
    return undefined;
  } finally {
    pendingOperations -= 1;
    sync.busy = pendingOperations > 0;
  }
}
