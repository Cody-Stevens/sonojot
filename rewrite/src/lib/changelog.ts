import type { WhatsNew } from "./protocol";

/** "12 November 2026" in the reader's locale; empty when unknown. */
export function releaseDate(date: string): string {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!match) return "";
  const value = new Date(
    Number(match[1]),
    Number(match[2]) - 1,
    Number(match[3]),
  );
  return value.toLocaleDateString(undefined, {
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}

/** One line above the notes: which versions they cover. */
export function changelogSummary(notes: WhatsNew | null): string {
  // One section names its own version; a range needs its starting point.
  if (!notes?.from || notes.releases.length < 2) return "";
  return `Changes since ${notes.from}.`;
}
