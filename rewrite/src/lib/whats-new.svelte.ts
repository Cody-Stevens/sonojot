import { call, native } from "./api";
import { errorMessage, type WhatsNew, type WhatsNewState } from "./protocol";

/** "installed": the update just installed (or this build). "offer": the update on offer. */
export type ChangelogMode = "installed" | "offer";

class WhatsNewStore {
  state = $state<WhatsNewState>({ pending: null, latest: null });
  mode = $state<ChangelogMode | null>(null);
  offer = $state<WhatsNew | null>(null);
  loading = $state(false);
  error = $state("");

  /** Notes for the installed build: the unseen update first, else the last one. */
  get installed(): WhatsNew | null {
    return this.state.pending ?? this.state.latest;
  }

  async check(): Promise<void> {
    if (native) this.state = await call("get_whats_new");
  }

  showInstalled(): void {
    this.mode = "installed";
  }

  async showOffer(): Promise<void> {
    this.mode = "offer";
    this.offer = null;
    this.error = "";
    if (!native) return;
    this.loading = true;
    try {
      this.offer = await call("get_update_changelog");
    } catch (failure: unknown) {
      this.error = errorMessage(failure);
    } finally {
      this.loading = false;
    }
  }

  /** Closing the unseen notes marks them seen; About can show them again. */
  async close(): Promise<void> {
    const closing = this.mode;
    this.mode = null;
    if (closing === "installed" && this.state.pending && native) {
      try {
        this.state = await call("dismiss_whats_new");
      } catch {
        // Shown again next launch; nothing is lost.
      }
    }
  }
}

export const whatsNew = new WhatsNewStore();
