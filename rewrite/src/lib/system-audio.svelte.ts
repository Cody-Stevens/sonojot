import { app } from "./app.svelte";
import { call } from "./api";
import { systemAudioFixable } from "./meeting-source";
import { openPermissionAssistant, watchPermission } from "./permissions";
import type { PermissionStatus, SystemAudioStatus } from "./protocol";

function publish(status: SystemAudioStatus): SystemAudioStatus {
  app.snapshot.capabilities.systemAudioStatus = status;
  app.snapshot.capabilities.systemAudio = status === "ready";
  return status;
}

/** Re-reads the OS permission; it can change in System Settings at any time. */
export async function refreshSystemAudio(): Promise<SystemAudioStatus> {
  if (!app.native) return app.snapshot.capabilities.systemAudioStatus;
  return publish(await call("system_audio_status"));
}

/**
 * The single "fix system audio" action used by every surface: the shared
 * permission assistant shows the macOS prompt when it was never answered, and
 * otherwise opens Screen & System Audio Recording with the drag helper.
 */
export async function fixSystemAudio(): Promise<PermissionStatus | null> {
  if (!app.native) return null;
  const status = await openPermissionAssistant("system_audio");
  await refreshSystemAudio();
  return status;
}

/**
 * Keeps the four-state status current: polls while it can still be fixed,
 * and follows the helper's permission events and window focus.
 */
export function watchSystemAudio(
  intervalMs = 1500,
  onPermission?: (status: PermissionStatus) => void,
): () => void {
  if (!app.native) return () => {};
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const refresh = () => refreshSystemAudio().catch(() => undefined);
  const tick = async () => {
    if (disposed) return;
    if (
      document.visibilityState === "visible" &&
      systemAudioFixable(app.snapshot.capabilities.systemAudioStatus)
    )
      await refresh();
    if (!disposed) timer = setTimeout(() => void tick(), intervalMs);
  };
  const stopPermission = watchPermission(
    "system_audio",
    (status) => {
      onPermission?.(status);
      void refresh();
    },
    () => {},
  );
  timer = setTimeout(() => void tick(), intervalMs);
  return () => {
    disposed = true;
    clearTimeout(timer);
    stopPermission();
  };
}
