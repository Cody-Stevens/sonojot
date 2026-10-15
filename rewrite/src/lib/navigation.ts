export type Page =
  | "dictation"
  | "microphone"
  | "vocabulary"
  | "meetings"
  | "appearance"
  | "shortcuts"
  | "report"
  | "about";
export type SetupPage = "setup";

/** Sidebar destinations. The first two sit at the top; the rest are Settings. */
export const navigation = [
  { id: "dictation", label: "Dictation" },
  { id: "meetings", label: "Meetings" },
  { id: "microphone", label: "Microphone" },
  { id: "shortcuts", label: "Shortcuts" },
  { id: "vocabulary", label: "Vocabulary" },
  { id: "appearance", label: "Appearance" },
  { id: "setup", label: "Models" },
  { id: "about", label: "About" },
  { id: "report", label: "Feedback" },
] satisfies { id: Page | SetupPage; label: string }[];
