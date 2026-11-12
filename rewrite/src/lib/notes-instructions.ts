import type { NotesPreset } from "./protocol";

// Match meeting_ai::NOTES_INSTRUCTIONS_LIMIT; count Unicode code points, not UTF-16 units.
export const NOTES_INSTRUCTIONS_LIMIT = 2_000;
export const instructionLength = (value: string): number => [...value].length;

export function notesInstructionsError(
  preset: NotesPreset | null,
  value: string,
): string {
  if (
    instructionLength(value) > NOTES_INSTRUCTIONS_LIMIT ||
    value.includes("\0")
  )
    return "Use at most 2,000 characters without null characters for note instructions.";
  return preset === "custom" && !value.trim()
    ? "Add instructions for custom notes."
    : "";
}
