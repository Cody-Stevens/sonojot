import { call, native } from "./api";
import type { AccelerationStatus } from "./acceleration";

class AccelerationState {
  data = $state<AccelerationStatus | null>(null);
  unavailable = $state(false);
  private users = 0;
  private running = false;
  private refreshAgain = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private readonly onVisibilityChange = (): void => {
    if (document.visibilityState === "visible") void this.refresh();
    else this.syncPolling();
  };
  private readonly onFocus = (): void => {
    void this.refresh();
  };

  async refresh(): Promise<void> {
    if (!native) return;
    if (this.running) {
      this.refreshAgain = true;
      return;
    }
    this.running = true;
    try {
      this.data = await call("get_model_acceleration");
      this.unavailable = false;
    } catch {
      // Do not keep displaying stale GPU proof after the host disconnects.
      this.data = null;
      this.unavailable = true;
    } finally {
      this.running = false;
      this.syncPolling();
      if (this.refreshAgain) {
        this.refreshAgain = false;
        if (this.users > 0) void this.refresh();
      }
    }
  }
  acquire(): () => void {
    this.users += 1;
    if (native && this.users === 1) {
      document.addEventListener("visibilitychange", this.onVisibilityChange);
      window.addEventListener("focus", this.onFocus);
      void this.refresh();
    }
    return () => {
      this.users = Math.max(0, this.users - 1);
      if (native && this.users === 0) {
        document.removeEventListener(
          "visibilitychange",
          this.onVisibilityChange,
        );
        window.removeEventListener("focus", this.onFocus);
        this.stopPolling();
      }
    };
  }
  private shouldPoll(): boolean {
    const runtime = this.data?.textRuntime;
    // Observe idle eviction while a model is resident. Stop once unloaded;
    // hidden windows and unmounted controls never poll.
    return !!runtime || !!this.data?.asrRuntime?.modelId;
  }
  private syncPolling(): void {
    const active =
      native &&
      this.users > 0 &&
      document.visibilityState === "visible" &&
      this.shouldPoll();
    if (!active) {
      this.stopPolling();
      return;
    }
    if (this.timer === undefined) {
      this.timer = setTimeout(
        () => {
          this.timer = undefined;
          void this.refresh();
        },
        this.data?.textRuntime?.loading ? 2000 : 5000,
      );
    }
  }
  private stopPolling(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
  }
}
export const acceleration = new AccelerationState();
