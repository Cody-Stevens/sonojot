import type { DictationRecord } from "./protocol";

export function dayPages(
  records: readonly DictationRecord[],
  query: string,
  now = new Date(),
) {
  const needle = query.trim().toLocaleLowerCase();
  const today = now.toDateString();
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  const groups = new Map<
    string,
    { key: string; label: string; date: string; records: DictationRecord[] }
  >();
  const sorted = [...records].sort(
    (a, b) =>
      b.createdAt.localeCompare(a.createdAt) || a.id.localeCompare(b.id),
  );
  for (const record of sorted) {
    if (
      needle &&
      ![
        record.published,
        record.raw,
        record.preModelText ?? "",
        record.targetApp ?? "",
      ].some((text) => text.toLocaleLowerCase().includes(needle))
    )
      continue;
    const time = new Date(record.createdAt);
    const valid = !Number.isNaN(time.getTime());
    const key = valid ? time.toDateString() : "unknown";
    if (!groups.has(key))
      groups.set(key, {
        key,
        label:
          key === today
            ? "Today"
            : key === yesterday.toDateString()
              ? "Yesterday"
              : valid
                ? time.toLocaleDateString(undefined, { weekday: "long" })
                : "Undated",
        date: valid
          ? time.toLocaleDateString(undefined, {
              day: "numeric",
              month: "short",
              year:
                time.getFullYear() !== now.getFullYear()
                  ? "numeric"
                  : undefined,
            })
          : "",
        records: [],
      });
    groups.get(key)!.records.push(record);
  }
  return [...groups.values()];
}

export function dictationTime(createdAt: string): string {
  const date = new Date(createdAt);
  return Number.isNaN(date.getTime())
    ? "—"
    : date.toLocaleTimeString(undefined, {
        hour: "2-digit",
        minute: "2-digit",
      });
}

export type DictationVersion = "published" | "pre_model" | "raw";
export function versionText(
  record: DictationRecord,
  version: DictationVersion,
): string | null {
  return version === "raw"
    ? record.raw
    : version === "pre_model"
      ? record.preModelText
      : record.published;
}

function phraseMatch(text: string, phrase: string): RegExpExecArray | null {
  if (!phrase.trim()) return null;
  const escaped = phrase.trim().replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(
    `(?<![\\p{L}\\p{N}_])${escaped}(?![\\p{L}\\p{N}_])`,
    "iu",
  ).exec(text);
}

export function hasPhrase(text: string, phrase: string): boolean {
  return phraseMatch(text, phrase) !== null;
}

/** A single explicit phrase replacement, without interpreting punctuation as a pattern. */
export function replaceWord(
  text: string,
  heard: string,
  written: string,
): string | null {
  const phrase = heard.trim();
  if (!phrase || !written.trim()) return null;
  const match = phraseMatch(text, phrase);
  return match === null
    ? null
    : text.slice(0, match.index) +
        written.trim() +
        text.slice(match.index + match[0].length);
}
