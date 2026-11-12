import type { UpdateStatus } from "./protocol";
import { formatBytes } from "./model-library";

export function updateStatusLabel(status: UpdateStatus): string {
  switch (status.state) {
    case "idle":
      return "Updates are checked automatically.";
    case "checking":
      return "Checking for updates…";
    case "available":
      if (status.repair)
        return `The last update didn't finish. Reinstall SonoJot ${status.version} to repair it.`;
      return status.older
        ? `SonoJot ${status.version} is the newest build on this channel.`
        : `SonoJot ${status.version} is available.`;
    case "downloading":
      return status.percent === null
        ? "Downloading update…"
        : `Downloading update · ${Math.floor(status.percent)}%`;
    case "ready":
      return `SonoJot ${status.version} is ready.`;
    case "error":
      return status.message;
  }
}

/** Installing the offer replaces this build with an older one. */
function switching(status: UpdateStatus): boolean {
  return (
    (status.state === "available" || status.state === "ready") && !!status.older
  );
}

export function updateButtonLabel(
  status: UpdateStatus,
  installing: boolean,
): string {
  if (installing) return "Restarting…";
  switch (status.state) {
    case "idle":
      return "";
    case "checking":
      return "Checking…";
    case "available":
      if (status.repair) return "Repair update";
      return status.older ? "Switch version" : "Update available";
    case "downloading":
      return status.percent === null
        ? "Downloading…"
        : `Downloading… ${Math.floor(status.percent)}%`;
    case "ready":
      return switching(status) ? "Restart to switch" : "Restart to update";
    case "error":
      return "Update failed · Retry";
  }
}

export function updateTooltip(
  status: UpdateStatus,
  installing: boolean,
  blockedReason: string | null,
): string {
  if (installing) return "Restarting to finish updating SonoJot.";
  switch (status.state) {
    case "idle":
      return "";
    case "checking":
      return "Checking for a new version of SonoJot.";
    case "available": {
      // Release metadata is plain text, never HTML. Keep the tooltip brief.
      const notes = status.notes?.replace(/\s+/g, " ").trim() ?? "";
      const summary =
        notes.length > 240 ? `${notes.slice(0, 237).trimEnd()}…` : notes;
      return [
        `SonoJot ${status.version}${status.size === null ? "" : ` · ${formatBytes(status.size)}`}.`,
        status.repair
          ? "The last update didn't finish, so this reinstalls the same version."
          : status.older
            ? "It's older than this build, so features added since then go away."
            : summary,
        "Click to download. Nothing changes until you restart.",
      ]
        .filter(Boolean)
        .join(" ");
    }
    case "downloading":
      return "Keep working. You'll be asked to restart when it's ready.";
    case "ready":
      return [
        "Click to restart and finish updating. Save any edits first.",
        blockedReason,
      ]
        .filter(Boolean)
        .join(" ");
    case "error":
      return `${status.message} Click to check for updates again.`;
  }
}

export type UpdateActionKind = "download" | "restart";

/**
 * The next step About and the main-window notice offer beside the status.
 * While downloading there is nothing to press; the status text beside it
 * carries the progress.
 */
export function updateAction(
  status: UpdateStatus,
  installing: boolean,
): { kind: UpdateActionKind; label: string } | null {
  if (installing) return { kind: "restart", label: "Restarting…" };
  switch (status.state) {
    case "available":
      return {
        kind: "download",
        label: status.repair
          ? "Download repair"
          : status.older
            ? `Download ${status.version}`
            : "Download update",
      };
    case "ready":
      return {
        kind: "restart",
        label: switching(status) ? "Restart to switch" : "Restart to update",
      };
    default:
      return null;
  }
}

/** Whether the shared update action can run now. */
export function updateActionDisabled(
  status: UpdateStatus,
  installing: boolean,
  blockedReason: string | null,
  native: boolean,
  busy: (key: string) => boolean,
): boolean {
  return (
    !native ||
    installing ||
    status.state === "checking" ||
    status.state === "downloading" ||
    busy("update-download") ||
    busy("update-check") ||
    busy("update-install") ||
    (status.state === "ready" && !!blockedReason)
  );
}

/**
 * The main window's update notice: shown while an update is available,
 * downloading or ready, until Later is chosen for that version. Errors and
 * checks stay in About and the sidebar.
 */
export function updateNoticeMessage(update: {
  status: UpdateStatus;
  installing: boolean;
  noticeDismissed: boolean;
}): string | null {
  if (update.noticeDismissed) return null;
  if (update.installing) return "Restarting to update…";
  switch (update.status.state) {
    case "available":
    case "downloading":
    case "ready":
      return updateStatusLabel(update.status);
    default:
      return null;
  }
}

export function restartBlockedReason(
  backendReason: string | null,
  dirty: boolean,
  pending: readonly string[],
): string | null {
  if (backendReason) return backendReason;
  if (dirty) return "Save or discard your edits before restarting.";
  if (pending.length) return "Wait for the current action to finish.";
  return null;
}
