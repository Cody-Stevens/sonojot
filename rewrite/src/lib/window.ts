import { getCurrentWindow, type Window } from "@tauri-apps/api/window";
import { native } from "./api";

export type WindowAction =
  "minimize" | "toggleMaximize" | "close" | "startDragging";
type WindowActions = Pick<Window, WindowAction>;

export async function dispatchWindowAction(
  target: WindowActions,
  action: WindowAction,
): Promise<void> {
  switch (action) {
    case "minimize":
      return target.minimize();
    case "toggleMaximize":
      return target.toggleMaximize();
    case "close":
      return target.close();
    case "startDragging":
      return target.startDragging();
  }
}

export async function controlWindow(action: WindowAction): Promise<void> {
  if (!native)
    throw new Error("Window controls are available in the desktop app.");
  await dispatchWindowAction(getCurrentWindow(), action);
}

export async function watchWindowMaximized(
  onChange: (value: boolean) => void,
  onError: (error: unknown) => void,
): Promise<() => void> {
  if (!native) return () => {};
  const current = getCurrentWindow();
  const refresh = async () => {
    try {
      onChange(await current.isMaximized());
    } catch (error: unknown) {
      onError(error);
    }
  };
  const unlisten = await current.onResized(() => {
    void refresh();
  });
  await refresh();
  return unlisten;
}

export function headerGesture(
  button: number,
  clickCount: number,
  interactive: boolean,
): "drag" | "maximize" | null {
  if (interactive || button !== 0) return null;
  return clickCount === 2 ? "maximize" : clickCount <= 1 ? "drag" : null;
}

/** Window dragging is a pointer affordance; adjacent buttons provide keyboard controls. */
export function draggableHeader(
  node: HTMLElement,
  options: { onMaximize: () => void; onError: (error: unknown) => void },
): { destroy: () => void } {
  const mouseDown = (event: MouseEvent) => {
    if (!native) return;
    const interactive =
      event.target instanceof Element &&
      Boolean(
        event.target.closest(
          'button,a,input,textarea,select,summary,[role="button"],[contenteditable],[data-no-window-drag]',
        ),
      );
    const gesture = headerGesture(event.button, event.detail, interactive);
    if (!gesture) return;
    event.preventDefault();
    if (gesture === "maximize") options.onMaximize();
    else void controlWindow("startDragging").catch(options.onError);
  };
  node.addEventListener("mousedown", mouseDown);
  return { destroy: () => node.removeEventListener("mousedown", mouseDown) };
}
