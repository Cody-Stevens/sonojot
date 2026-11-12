export const tabs = [
  { id: "dictation", label: "Dictation" },
  { id: "meetings", label: "Meetings" },
  { id: "words", label: "Words" },
  { id: "settings", label: "Settings" },
] as const;
export type Tab = (typeof tabs)[number]["id"];
type Origin = { from?: "settings" };
export type Route = Origin &
  (
    | {
        tab: "dictation";
        stage?: "input" | "speech" | "cleanup" | "output";
        item?: "microphone" | "shortcuts";
        sheet?: never;
      }
    | {
        tab: "meetings";
        stage?: "speech" | "notes";
        item?: string;
        sheet?: never;
      }
    | { tab: "words"; stage?: never; item?: string; sheet?: never }
    | {
        tab: "settings";
        stage?:
          | "dictation"
          | "meetings"
          | "look"
          | "memory"
          | "disk"
          | "updates"
          | "help";
        item?: never;
        sheet?: "report" | "update";
      }
  );

/** Native menus, show_page, error actions and old bookmarks keep these ids. */
export const legacyRoutes = {
  dictation: { tab: "dictation" },
  meetings: { tab: "meetings" },
  setup: { tab: "dictation", stage: "speech" },
  microphone: { tab: "dictation", stage: "input", item: "microphone" },
  shortcuts: { tab: "dictation", stage: "input", item: "shortcuts" },
  vocabulary: { tab: "words" },
  appearance: { tab: "settings", stage: "look" },
  about: { tab: "settings", stage: "help" },
  report: { tab: "settings", stage: "help", sheet: "report" },
} as const satisfies Record<string, Route>;
export type LegacyPage = keyof typeof legacyRoutes;
export type RouteTarget = Route | LegacyPage | Tab;

export function resolveRoute(
  target: RouteTarget | string | null,
): Route | null {
  if (target === null) return null;
  if (typeof target !== "string") return { ...target };
  if (Object.hasOwn(legacyRoutes, target))
    return { ...legacyRoutes[target as LegacyPage] };
  if (target === "words" || target === "settings") return { tab: target };
  return null;
}

export function sameRoute(a: Route, b: Route): boolean {
  return (
    a.tab === b.tab &&
    a.stage === b.stage &&
    a.sheet === b.sheet &&
    a.item === b.item &&
    a.from === b.from
  );
}

export function backRoute(route: Route): Route {
  if (route.sheet) return { ...route, sheet: undefined };
  if (route.from === "settings" || route.tab === "settings")
    return { tab: "settings" };
  return { tab: route.tab };
}

export function routeTitle(route: Route): string {
  if (route.sheet === "report") return "Report a problem";
  if (route.sheet === "update") return "Updates";
  if (route.stage)
    return {
      input: "Shortcut and mic",
      speech: "Speech",
      cleanup: "Cleanup",
      output: "Paste and history",
      notes: "Notes",
      dictation: "Dictation",
      meetings: "Meetings",
      look: "Look and startup",
      memory: "Memory",
      disk: "On disk",
      updates: "Updates",
      help: "Help",
    }[route.stage];
  return tabs.find((tab) => tab.id === route.tab)?.label ?? "Dictation";
}

/** One guarded transition path, including sheets and native navigation events. */
export function createRouter(
  initial: Route,
  beforeLeave: (next: Route) => Promise<boolean>,
) {
  let current = $state<Route>(initial);
  let changing = false;
  return {
    get current() {
      return current;
    },
    async go(target: RouteTarget | string): Promise<boolean> {
      const next = resolveRoute(target);
      if (!next || changing) return false;
      if (sameRoute(current, next)) return true;
      changing = true;
      try {
        if (!(await beforeLeave(next))) return false;
        current = next;
        return true;
      } finally {
        changing = false;
      }
    },
  };
}
