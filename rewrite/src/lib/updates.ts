import type { PackageManager, UpdateStatus } from "./protocol";
import { formatBytes } from "./model-library";

function channelName(channel: "stable" | "nightly"): string {
  return channel === "nightly" ? "Nightly" : "Stable";
}

/** A Linux package format as people name it, with its article. */
function packageName(manager: PackageManager): string {
  return manager === "dpkg"
    ? "a Debian package"
    : manager === "rpm"
      ? "an RPM package"
      : "a pacman package";
}

/** A Linux package manager installs this update, asking for a password. */
function packageInstall(status: UpdateStatus): boolean {
  return status.state === "ready" && !!status.package;
}

/** Shown while the update installs and SonoJot restarts. */
export function updateRestartLabel(status: UpdateStatus): string {
  if (status.state === "ready" && status.package) {
    if (status.prompt === "password")
      return `Enter your password to install SonoJot ${status.version}.`;
    if (status.prompt === "terminal")
      return `Enter your password in the terminal window to install SonoJot ${status.version}.`;
    return `Installing SonoJot ${status.version}…`;
  }
  return switching(status) ? "Restarting to switch…" : "Restarting to update…";
}

export function updateStatusLabel(status: UpdateStatus): string {
  switch (status.state) {
    case "idle":
      return "Updates are checked automatically.";
    case "checking":
      return "Checking for updates…";
    case "current": {
      const channel = channelName(status.channel);
      return status.reason === "current"
        ? `You're already on the newest ${channel} build.`
        : `${channel} has no ${status.platform} build newer than ${status.version} yet. You'll get ${channel} builds here when they're published.`;
    }
    case "available":
      if (status.repair)
        return `The last update didn't finish. Reinstall SonoJot ${status.version} to repair it.`;
      if (status.switchChannel)
        return `Switch to ${channelName(status.switchChannel)}: ${status.version} is available.`;
      return status.older
        ? `SonoJot ${status.version} is the newest build on this channel.`
        : `SonoJot ${status.version} is available.`;
    case "manual": {
      const offer = status.switchChannel
        ? `Switch to ${channelName(status.switchChannel)}: ${status.version} is available`
        : `SonoJot ${status.version} is available`;
      return status.reason === "noPackage" && status.package
        ? `${offer}, but not yet as ${packageName(status.package)}. Get it from the download page.`
        : `${offer}. This copy can't update in the app; get it from the download page.`;
    }
    case "downloading":
      if (status.switchChannel)
        return status.percent === null
          ? `Downloading ${channelName(status.switchChannel)} to switch…`
          : `Downloading ${channelName(status.switchChannel)} to switch · ${Math.floor(status.percent)}%`;
      return status.percent === null
        ? "Downloading update…"
        : `Downloading update · ${Math.floor(status.percent)}%`;
    case "ready":
      if (status.package) {
        if (status.installError) return status.installError;
        const name = status.switchChannel
          ? `${channelName(status.switchChannel)} ${status.version}`
          : `SonoJot ${status.version}`;
        return `${name} is ready. Installing it asks for your password.`;
      }
      if (status.switchChannel)
        return `${channelName(status.switchChannel)} ${status.version} is ready. Restart to switch.`;
      return `SonoJot ${status.version} is ready.`;
    case "error":
      return status.message;
  }
}

/**
 * An update is on offer: available (in the app or from the download page),
 * downloading or ready to restart.
 */
export function updateOffered(status: UpdateStatus): boolean {
  return (
    status.state === "available" ||
    status.state === "manual" ||
    status.state === "downloading" ||
    status.state === "ready"
  );
}

/** A channel-change offer stays a switch even when its version is newer. */
function switching(status: UpdateStatus): boolean {
  return (
    ("switchChannel" in status && !!status.switchChannel) ||
    ((status.state === "available" || status.state === "ready") &&
      !!status.older)
  );
}

export function updateButtonLabel(
  status: UpdateStatus,
  installing: boolean,
): string {
  if (installing) return packageInstall(status) ? "Installing…" : "Restarting…";
  switch (status.state) {
    case "idle":
    case "current":
      return "";
    case "checking":
      return "Checking…";
    case "available":
      if (status.repair) return "Repair update";
      if (status.switchChannel)
        return `Switch to ${channelName(status.switchChannel)}`;
      return status.older ? "Switch version" : "Update available";
    case "manual":
      return status.switchChannel
        ? `Switch to ${channelName(status.switchChannel)}`
        : "Update available";
    case "downloading":
      return status.percent === null
        ? "Downloading…"
        : `Downloading… ${Math.floor(status.percent)}%`;
    case "ready":
      if (status.package)
        return switching(status) ? "Install to switch" : "Install update";
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
  if (installing)
    return packageInstall(status)
      ? updateRestartLabel(status)
      : switching(status)
        ? "Restarting to switch…"
        : "Restarting to finish updating SonoJot.";
  switch (status.state) {
    case "idle":
      return "";
    case "current":
      return updateStatusLabel(status);
    case "checking":
      return "Checking for a new version of SonoJot.";
    case "manual":
      return [updateStatusLabel(status), noteSummary(status.notes)]
        .filter(Boolean)
        .join(" ");
    case "available": {
      const summary = noteSummary(status.notes);
      return [
        status.switchChannel
          ? updateStatusLabel(status)
          : `SonoJot ${status.version}${status.size === null ? "" : ` · ${formatBytes(status.size)}`}.`,
        status.repair
          ? "The last update didn't finish, so this reinstalls the same version."
          : status.older
            ? "It's older than this build, so features added since then go away."
            : summary,
        status.package
          ? "Click to download. Nothing changes until you install it."
          : "Click to download. Nothing changes until you restart.",
      ]
        .filter(Boolean)
        .join(" ");
    }
    case "downloading":
      return "Keep working. You'll be asked to restart when it's ready.";
    case "ready":
      return [
        status.package
          ? status.installError
            ? `${status.installError} Click to try again.`
            : switching(status)
              ? "Click to install and switch channels. SonoJot asks for your password, then restarts. Save any edits first."
              : "Click to install. SonoJot asks for your password, then restarts. Save any edits first."
          : switching(status)
            ? "Click to restart and switch channels. Save any edits first."
            : "Click to restart and finish updating. Save any edits first.",
        blockedReason,
      ]
        .filter(Boolean)
        .join(" ");
    case "error":
      return `${status.message} Click to check for updates again.`;
  }
}

/** Release metadata is plain text, never HTML. Keep tooltips brief. */
function noteSummary(notes: string | null | undefined): string {
  const text = notes?.replace(/\s+/g, " ").trim() ?? "";
  return text.length > 240 ? `${text.slice(0, 237).trimEnd()}…` : text;
}

/** Restart installs the update; open shows the website's download page. */
export type UpdateActionKind = "download" | "restart" | "open";

/**
 * The next step About and the main-window notice offer beside the status.
 * While downloading there is nothing to press; the status text beside it
 * carries the progress.
 */
export function updateAction(
  status: UpdateStatus,
  installing: boolean,
): { kind: UpdateActionKind; label: string } | null {
  if (installing)
    return {
      kind: "restart",
      label: packageInstall(status) ? "Installing…" : "Restarting…",
    };
  switch (status.state) {
    case "available":
      return {
        kind: "download",
        label: status.repair
          ? "Download repair"
          : switching(status)
            ? `Download ${status.version}`
            : "Download update",
      };
    case "manual":
      return { kind: "open", label: "Open download page" };
    case "ready":
      return {
        kind: "restart",
        label: status.package
          ? switching(status)
            ? "Install to switch"
            : "Install update"
          : switching(status)
            ? "Restart to switch"
            : "Restart to update",
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
  if (update.installing) return updateRestartLabel(update.status);
  switch (update.status.state) {
    case "available":
    case "manual":
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
