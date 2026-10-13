export type TranscriptSide = "you" | "other" | "unknown";

export type TranscriptBlock = {
  id: string;
  speaker: string;
  side: TranscriptSide;
  timecode: string | null;
  text: string;
};

// Only known labels and clock-shaped brackets count as structure; ordinary
// prose such as "The plan is simple: ship Friday" or "[laughter]" stays text.
const clock = String.raw`\d{1,3}:\d{2}(?::\d{2})?`;
const labels = String.raw`you|me|microphone|local speaker|other speakers?|system audio|remote speaker|speaker \d{1,2}`;
const speakerLine = new RegExp(String.raw`^\s*(?:\[(${clock})\]\s*)?(${labels}):\s*(.*)$`, "i");
const timecode = new RegExp(String.raw`^\s*\[(${clock})\]\s*`);

function sideFor(speaker: string): TranscriptSide {
  const value = speaker.toLocaleLowerCase();
  if (/^(you|me|microphone|local speaker)$/.test(value)) return "you";
  if (/^(other speakers?|system audio|remote speaker)$/.test(value)) return "other";
  return "unknown";
}

export function parseTranscript(value: string): TranscriptBlock[] {
  try {
    const parsed: unknown = JSON.parse(value);
    const segments = Array.isArray(parsed)
      ? parsed
      : parsed && typeof parsed === "object" && "segments" in parsed
        ? parsed.segments
        : null;
    if (Array.isArray(segments)) {
      const structured = segments.flatMap((segment, index) => {
        if (!segment || typeof segment !== "object" || !("text" in segment)) return [];
        const item = segment as Record<string, unknown>;
        const speaker = typeof item.speaker === "string" ? item.speaker : "Transcript";
        const seconds = typeof item.startSeconds === "number" ? item.startSeconds : null;
        const time = typeof item.timecode === "string"
          ? item.timecode
          : seconds === null
            ? null
            : `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`;
        return [{
          id: `block-${index}`,
          speaker,
          side: item.side === "you" || item.side === "other" ? item.side : sideFor(speaker),
          timecode: time,
          text: String(item.text),
        } satisfies TranscriptBlock];
      });
      if (structured.length) return structured;
    }
  } catch {
    // Existing meetings store plain transcript text; fall through to that format.
  }
  const legacy = parseLegacyTable(value);
  if (legacy) return legacy;
  const paragraphs = value
    .split(/\n\s*\n/)
    .map((text) => text.trim())
    .filter(Boolean);
  if (!paragraphs.length && value.trim()) paragraphs.push(value.trim());

  return paragraphs.map((paragraph, index) => {
    const line = paragraph.split("\n")[0] ?? paragraph;
    const match = line.match(speakerLine);
    if (match) {
      return {
        id: `block-${index}`,
        speaker: match[2].trim(),
        side: sideFor(match[2]),
        timecode: match[1] ?? null,
        text: [match[3], ...paragraph.split("\n").slice(1)].join("\n").trim(),
      };
    }
    const timestamp = paragraph.match(timecode);
    return {
      id: `block-${index}`,
      speaker: "Transcript",
      side: "unknown",
      timecode: timestamp?.[1] ?? null,
      text: paragraph.replace(timecode, "").trim(),
    };
  });
}

/**
 * The legacy app saved two-source meetings as a tab-separated table of
 * ten-second rows: `Time\tOther speakers\tMicrophone`. Consecutive rows from
 * one side join into a passage that keeps the clock of its first row.
 */
function parseLegacyTable(value: string): TranscriptBlock[] | null {
  const [header, ...rows] = value.split(/\r?\n/);
  const columns = header.split("\t").map((cell) => cell.trim().toLowerCase());
  if (columns.join("|") !== "time|other speakers|microphone") return null;
  const stamp = new RegExp(`^${clock}$`);
  const blocks: TranscriptBlock[] = [];
  for (const row of rows) {
    const [time = "", other = "", microphone = ""] = row.split("\t");
    for (const [speaker, cell] of [
      ["Other speakers", other],
      ["You", microphone],
    ] as const) {
      const text = cell.trim();
      if (!text) continue;
      const last = blocks[blocks.length - 1];
      if (last?.speaker === speaker) last.text += ` ${text}`;
      else
        blocks.push({
          id: `block-${blocks.length}`,
          speaker,
          side: sideFor(speaker),
          timecode: stamp.test(time.trim()) ? time.trim() : null,
          text,
        });
    }
  }
  return blocks.length ? blocks : null;
}

/** Plain recognition text has no speakers or timecodes and keeps the single document editor. */
export function hasStructure(blocks: TranscriptBlock[]): boolean {
  return blocks.some((block) => block.speaker !== "Transcript" || block.timecode !== null);
}

export function serializeTranscript(blocks: TranscriptBlock[]): string {
  return blocks
    .map((block) => {
      const prefix = block.speaker === "Transcript" ? "" : `${block.speaker}: `;
      const timestamp = block.timecode ? `[${block.timecode}] ` : "";
      return `${timestamp}${prefix}${block.text.trim()}`.trim();
    })
    .filter(Boolean)
    .join("\n\n");
}
