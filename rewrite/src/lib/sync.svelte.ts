import { invoke, isTauri } from "@tauri-apps/api/core";
import { z } from "zod";
import { app } from "./app.svelte";
const statusSchema = z.object({
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
}>({ status: null, busy: false, error: "" });
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
    if (sync.status.error) sync.error = sync.status.error;
  } catch (error) {
    sync.error = String(error);
  }
}
export async function runSync(
  operation: string,
  args: Record<string, unknown> = {},
): Promise<unknown> {
  sync.busy = true;
  sync.error = "";
  try {
    const result = await syncAction(operation, args);
    await refreshSync();
    return result;
  } catch (error) {
    sync.error = String(error);
    return undefined;
  } finally {
    sync.busy = false;
  }
}
