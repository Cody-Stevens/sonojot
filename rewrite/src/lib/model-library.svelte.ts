import { call, native } from "./api";
import { errorMessage } from "./protocol";
import { checking, installing, type ModelLibrary } from "./model-library";

class LibraryState {
  data = $state<ModelLibrary | null>(null);
  error = $state("");
  loading = $state(false);
  downloadId = $state("");
  private users = 0;
  private version = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private readonly onVisibilityChange = (): void => {
    if (document.visibilityState === "visible") void this.refresh();
    else this.syncPolling();
  };
  private readonly onFocus = (): void => {
    void this.refresh();
  };

  apply(data: ModelLibrary): void {
    this.version++;
    this.data = data;
    const download = data.models.find(installing);
    if (download) this.downloadId = download.id;
    this.error = "";
    this.syncPolling();
  }
  async refresh(): Promise<void> {
    if (!native || this.loading) return;
    this.loading = true;
    const version = this.version;
    try {
      const data = await call("get_model_library");
      if (version === this.version) this.apply(data);
    } catch (error: unknown) {
      if (version === this.version) {
        this.data = null;
        this.error = errorMessage(error);
      }
    } finally {
      this.loading = false;
      this.syncPolling();
    }
  }
  acquire(): () => void {
    this.users++;
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
    return !!this.data?.models.some(
      (model) =>
        checking(model) ||
        installing(model) ||
        ["loading", "unloading", "busy"].includes(model.state),
    );
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
      this.timer = setTimeout(() => {
        this.timer = undefined;
        void this.refresh();
      }, 2000);
    }
  }
  private stopPolling(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
  }
}
export const library = new LibraryState();
