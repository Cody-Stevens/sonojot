export type OnlineRole = "cleanup" | "notes" | "meeting" | "manage";

class OpenRouterState {
  request = $state<{ role: OnlineRole; model?: string } | null>(null);
  open(role: OnlineRole, model?: string) {
    this.request = { role, model };
  }
  close() {
    this.request = null;
  }
}

export const openRouter = new OpenRouterState();
