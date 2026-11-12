import type { VocabularyEntry } from "./protocol";

export function validHeardForm(value: string): boolean {
  return (
    new TextEncoder().encode(value).length <= 250 &&
    /^[\p{L}\p{N}](?:[\p{L}\p{N} .-]*[\p{L}\p{N}])?$/u.test(value) &&
    value.split(/\s+/).length <= 4
  );
}

export function parseWord(input: string): Omit<VocabularyEntry, "id"> {
  const parts = input.split(/→|->/).map((part) => part.trim());
  if (parts.length > 2 || !parts[0] || (parts.length === 2 && !parts[1]))
    throw new Error("Enter a word, or heard → written.");
  const [heard, written = heard] = parts;
  return wordEntry(heard, written, parts.length === 2 ? "replace" : "hint");
}

export function replacementWord(
  heard: string,
  written: string,
): Omit<VocabularyEntry, "id"> {
  return wordEntry(heard.trim(), written.trim(), "replace");
}

function wordEntry(
  heard: string,
  written: string,
  kind: "replace" | "hint",
): Omit<VocabularyEntry, "id"> {
  if (!heard || !written)
    throw new Error("Enter both a heard and written form.");
  if (/[\u0000-\u001f\u007f]/u.test(written))
    throw new Error("Words cannot contain control characters.");
  if (kind === "replace" && !validHeardForm(heard))
    throw new Error(
      "Use a heard form of 1–4 words, with letters, numbers, spaces, dots or hyphens.",
    );
  if (new TextEncoder().encode(written).length > 250)
    throw new Error("Keep the written form within 250 bytes.");
  // Explicit replacements confirm a heard form; hints remain context only.
  return {
    kind,
    heard,
    written,
    enabled: true,
    aliases: kind === "replace" ? [heard] : [],
    context: [],
    scope: null,
  };
}

export function wordKey(value: string): string {
  return value.trim().toLowerCase();
}
