const modifier =
  /^(ctrl|control|alt|option|super|meta|cmd|command|cmdorctrl|cmdorcontrol|commandorctrl|commandorcontrol|shift)$/i;
export const shortcutHelp =
  "Use Ctrl, Alt or Super with a key, or F1–F24. Esc keeps your shortcut.";

/** Shift alone must not turn an ordinary typing key into a global shortcut. */
export function safeShortcut(value: string): boolean {
  if (
    !value.trim() ||
    value.length > 128 ||
    /[\u0000-\u001f\u007f]/u.test(value)
  )
    return false;
  const parts = value.split("+").map((part) => part.trim());
  const key = parts.pop() ?? "";
  return (
    !!key &&
    !modifier.test(key) &&
    parts.every((part) => modifier.test(part)) &&
    (parts.some((part) => part.toLowerCase() !== "shift") ||
      /^F([1-9]|1\d|2[0-4])$/i.test(key))
  );
}
