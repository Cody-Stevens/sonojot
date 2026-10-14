import { listen } from "@tauri-apps/api/event";
import { call, native } from "./api";
import {
  permissionStatusSchema,
  type PermissionKind,
  type PermissionStatus,
} from "./protocol";

/**
 * macOS permission assistant (src-tauri/src/permissions.rs). Reusable for any
 * permission registered there:
 * - `openPermissionAssistant(kind)` opens its System Settings pane and the
 *   floating drag helper (`?view=permission&kind=…`), or reports it granted.
 * - `watchPermission(kind, onStatus)` reports the current status, every
 *   `permission-status` event for that kind, and a recheck when this window
 *   regains focus. It returns a cleanup function.
 */
export type PermissionState = "unknown" | "granted" | "needed" | "assisting";

export function permissionState(
  status: PermissionStatus | null,
): PermissionState {
  if (!status) return "unknown";
  if (!status.required || status.granted) return "granted";
  return status.assistantOpen ? "assisting" : "needed";
}

export async function openPermissionAssistant(
  kind: PermissionKind,
): Promise<PermissionStatus> {
  return call("permission_assistant", { kind, action: "open" });
}

export async function closePermissionAssistant(
  kind: PermissionKind,
): Promise<PermissionStatus> {
  return call("permission_assistant", { kind, action: "close" });
}

export function watchPermission(
  kind: PermissionKind,
  onStatus: (status: PermissionStatus) => void,
  onError: (error: unknown) => void,
): () => void {
  if (!native) return () => {};
  let disposed = false;
  let unlisten: (() => void) | undefined;
  const refresh = (): void => {
    call("permission_status", { kind })
      .then((status) => {
        if (!disposed) onStatus(status);
      })
      .catch(onError);
  };
  refresh();
  void listen<unknown>("permission-status", ({ payload }) => {
    const parsed = permissionStatusSchema.safeParse(payload);
    if (parsed.success && parsed.data.kind === kind && !disposed)
      onStatus(parsed.data);
  }).then((stop) => {
    if (disposed) stop();
    else unlisten = stop;
  });
  // Access switched on in System Settings without the helper is seen on return.
  window.addEventListener("focus", refresh);
  return () => {
    disposed = true;
    unlisten?.();
    window.removeEventListener("focus", refresh);
  };
}

/** Copy for automatic paste on macOS, shared by setup and the Dictation page. */
export const MAC_PASTE_COPY = {
  on: "Automatic paste is on.",
  needed:
    "To paste into the app you were using, SonoJot needs Accessibility access. Until then, text is copied; press Command+V to paste.",
  grant: "Grant Accessibility",
  assistant:
    "In System Settings, switch SonoJot on. If it isn't listed, drag the icon from the SonoJot helper into the list. If it is already on, remove it with − and add it again.",
  rowLabel: "Accessibility access",
  rowOn: "On. SonoJot can paste into the app you were using.",
  rowNeeded: "Needed to paste on a Mac. Until then, press Command+V to paste.",
} as const;

/** The helper offers the stale-entry fix after this long without access. */
export const STALE_HINT_DELAY_MS = 6000;
