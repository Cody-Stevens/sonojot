import type { NotesPreset } from "./protocol";

export const notesStyles: { id: NotesPreset; label: string }[] = [
  { id: "summary", label: "Summary" },
  { id: "tasks", label: "Tasks by person" },
  { id: "speakers", label: "By speaker" },
  { id: "brief", label: "Executive brief" },
  { id: "formattedTranscript", label: "Corrected transcript" },
  { id: "custom", label: "Custom" },
];
