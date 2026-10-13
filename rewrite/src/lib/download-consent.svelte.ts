import { call } from "./api";
import { app } from "./app.svelte";
import type { DownloadConsent } from "./download-consent";

class DownloadConsentState {
  groups = $state<DownloadConsent[]>([]);
  private finish: ((accepted: boolean) => void) | null = null;
  private busy = false;

  async request(ids: string[]): Promise<boolean> {
    // Another install owns its dialog until it is saved or cancelled.
    if (this.busy) return false;
    this.busy = true;
    try {
      const groups = await call("get_download_requirements", { ids });
      if (groups.length === 0) return true;
      this.groups = groups;
      return await new Promise<boolean>((resolve) => {
        this.finish = resolve;
      });
    } finally {
      this.groups = [];
      this.finish = null;
      this.busy = false;
    }
  }

  async accept(): Promise<void> {
    const snapshot = await call("acknowledge_download_terms", {
      acknowledgments: this.groups.map(({ id, version }) => ({ id, version })),
    });
    app.apply(snapshot);
    this.finish?.(true);
  }

  cancel(): void {
    this.finish?.(false);
  }
}

export const downloadConsent = new DownloadConsentState();
export const ensureDownloadConsent = (ids: string[]): Promise<boolean> =>
  downloadConsent.request(ids);
