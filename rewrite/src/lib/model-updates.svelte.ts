import { call, native } from "./api";
import { app } from "./app.svelte";
import { ensureDownloadConsent } from "./download-consent.svelte";
import { library } from "./model-library.svelte";
import { updateRunning, type ModelUpdate } from "./model-updates";

const IDLE_POLL_MS = 5 * 60 * 1000;
const ACTIVE_POLL_MS = 2000;

class ModelUpdatesState {
  list = $state<ModelUpdate[]>([]);
  /** Later: hidden until SonoJot next starts, or a different update appears. */
  dismissed = $state<string[]>([]);
  private timer: ReturnType<typeof setTimeout> | undefined;
  private users = 0;
  private running = new Map<string, string>();

  apply(list: ModelUpdate[]): void {
    // A running update that disappears has finished: say so once.
    const remaining = new Set(list.map((update) => update.id));
    for (const [id, name] of this.running) {
      if (!remaining.has(id)) {
        app.notice = `${name} updated.`;
        void library.refresh();
      }
    }
    this.running = new Map(
      list.filter(updateRunning).map((update) => [update.id, update.name]),
    );
    this.list = list;
    this.schedule();
  }

  async refresh(): Promise<void> {
    if (!native) return;
    try {
      this.apply(await call("get_model_updates"));
    } catch {
      // Offline or busy: no update is shown, nothing nags.
      this.schedule();
    }
  }

  /** Start one update, or every waiting one when `id` is omitted. */
  async start(id?: string): Promise<void> {
    const ids = id
      ? [id]
      : this.list.filter((update) => !update.automatic).map((u) => u.id);
    if (!(await ensureDownloadConsent(ids))) return;
    await app.perform("model-update", async () => {
      this.apply(await call("start_model_update", { id: id ?? null }));
    });
  }

  async cancel(id: string): Promise<void> {
    await app.perform("model-update", async () => {
      this.apply(await call("cancel_model_update", { id }));
    });
  }

  dismiss(ids: string[]): void {
    this.dismissed = [...new Set([...this.dismissed, ...ids])];
  }

  acquire(): () => void {
    this.users++;
    if (this.users === 1) void this.refresh();
    return () => {
      this.users = Math.max(0, this.users - 1);
      if (this.users === 0 && this.timer !== undefined) {
        clearTimeout(this.timer);
        this.timer = undefined;
      }
    };
  }

  private schedule(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    if (!native || this.users === 0) return;
    const active = this.list.some(updateRunning);
    this.timer = setTimeout(
      () => {
        this.timer = undefined;
        void this.refresh();
      },
      active ? ACTIVE_POLL_MS : IDLE_POLL_MS,
    );
  }
}

export const modelUpdates = new ModelUpdatesState();
