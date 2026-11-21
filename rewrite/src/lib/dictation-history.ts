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

function phraseMatches(text: string, phrase: string): RegExpExecArray[] {
  if (!phrase.trim()) return [];
  const escaped = phrase.trim().replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const re = new RegExp(
    `(?<![\\p{L}\\p{N}_])${escaped}(?![\\p{L}\\p{N}_])`,
    "giu",
  );
  const out: RegExpExecArray[] = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    out.push(m);
    if (m[0].length === 0) break;
  }
  return out;
}

export function hasPhrase(text: string, phrase: string): boolean {
  return phraseMatches(text, phrase).length > 0;
}

/** A single explicit phrase replacement, targeting the occurrence nearest `offset` when given. */
export function replaceWordAt(
  text: string,
  heard: string,
  written: string,
  offset?: number | null,
): string | null {
  const phrase = heard.trim();
  if (!phrase || !written.trim()) return null;
  const matches = phraseMatches(text, phrase);
  if (!matches.length) return null;
  let chosen = matches[0];
  if (typeof offset === "number" && offset >= 0) {
    let bestDist = Math.abs(chosen.index - offset);
    for (const m of matches) {
      const dist = Math.abs(m.index - offset);
      if (dist < bestDist) {
        chosen = m;
        bestDist = dist;
      }
    }
  }
  return (
    text.slice(0, chosen.index) +
    written.trim() +
    text.slice(chosen.index + chosen[0].length)
  );
}

/** A single explicit phrase replacement, without interpreting punctuation as a pattern. */
export function replaceWord(
  text: string,
  heard: string,
  written: string,
): string | null {
  return replaceWordAt(text, heard, written, null);
}

export function nearbyContextWords(text: string, phrase: string): string[] {
  const matches = phraseMatches(text, phrase);
  if (!matches.length) return [];
  const m = matches[0];
  const before = text
    .slice(Math.max(0, m.index - 48), m.index)
    .split(/[^a-zA-Z0-9]+/)
    .filter((w) => w.length >= 4);
  const after = text
    .slice(m.index + m[0].length, m.index + m[0].length + 48)
    .split(/[^a-zA-Z0-9]+/)
    .filter((w) => w.length >= 4);
  const stop = new Set([
    "that",
    "this",
    "with",
    "from",
    "have",
    "been",
    "were",
    "they",
    "them",
    "their",
    "there",
    "what",
    "when",
    "where",
    "which",
    "about",
    "into",
    "just",
    "like",
    "make",
    "more",
    "some",
    "than",
    "then",
    "very",
    "your",
  ]);
  const out: string[] = [];
  for (const w of [...before.slice(-3), ...after.slice(0, 3)]) {
    const low = w.toLowerCase();
    if (!stop.has(low) && !out.includes(low)) out.push(low);
  }
  return out.slice(0, 3);
}
