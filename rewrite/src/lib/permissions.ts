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
 * - `resetPermission(kind)` clears SonoJot's own entry (one left by an older
 *   build looks switched on but grants nothing), asks macOS again and
 *   reopens the pane with the helper.
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

export async function resetPermission(
  kind: PermissionKind,
): Promise<PermissionStatus> {
  return call("permission_reset", { kind });
}

/** Offer the reset once the user has been through the flow without access. */
export function offerReset(
  status: PermissionStatus | null,
  asked: boolean,
): boolean {
  return (
    asked && Boolean(status?.canReset) && permissionState(status) !== "granted"
  );
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

/** Clears an entry left by an older build; see `resetPermission`. */
export const PERMISSION_RESET_LABEL = "Reset and ask again";

/** Copy for automatic paste on macOS, shared by setup and the Dictation page. */
export const MAC_PASTE_COPY = {
  on: "Automatic paste is on.",
  needed:
    "To paste into the app you were using, SonoJot needs Accessibility access. Until then, text is copied; press Command+V to paste.",
  grant: "Grant Accessibility",
  assistant:
    "In System Settings, switch SonoJot on. If it isn't listed, click + and choose SonoJot in Applications, or drag the icon from the SonoJot helper into the list.",
  reset: PERMISSION_RESET_LABEL,
  resetHelp:
    "If SonoJot is already switched on but paste still doesn't work, reset it. macOS will ask again; switch SonoJot on.",
  rowLabel: "Accessibility access",
  rowOn: "On. SonoJot can paste into the app you were using.",
  rowNeeded: "Needed to paste on a Mac. Until then, press Command+V to paste.",
} as const;

/** The setup Permissions step: one row per macOS permission, in this order. */
export const SETUP_PERMISSIONS: {
  kind: PermissionKind;
  name: string;
  why: string;
}[] = [
  {
    kind: "microphone",
    name: "Microphone",
    why: "Hears you for dictation and meetings.",
  },
  {
    kind: "accessibility",
    name: "Paste into apps",
    why: "Pastes where your cursor is. Without it, press Command+V.",
  },
  {
    kind: "system_audio",
    name: "System audio",
    why: "Records the other people on a call.",
  },
];

/** Status words shared by the rows. */
export function permissionLabel(state: PermissionState): string {
  return state === "granted"
    ? "Allowed"
    : state === "assisting"
      ? "Finish in System Settings"
      : state === "unknown"
        ? "Checking…"
        : "Not allowed yet";
}

/** The helper offers the reset after this long without access. */
export const STALE_HINT_DELAY_MS = 6000;
