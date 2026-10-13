import type { DictationRecord, Settings, Snapshot } from "./protocol";

export const SMART_CLEANUP_MODEL = "speakoflow-mini-q4_k_m";
export const SMART_CLEANUP_HELP =
  "Small cleanup model; may miss or occasionally change wording. Numbers, names and links are checked.";

export function smartCleanupAvailable(settings: Settings): boolean {
  return settings.setupProfile === "custom";
}

export function smartRecoverySession(snapshot: Snapshot): number | null {
  return snapshot.resultSessionId !== null &&
    snapshot.sessionId === null &&
    !snapshot.meetingRecording &&
    ["idle", "error"].includes(snapshot.state) &&
    snapshot.recognitionCompleteness === "complete" &&
    (snapshot.publicationStage === "smart_cleanup" ||
      snapshot.publicationStage === "smart_layout") &&
    !!snapshot.preModelText?.trim() &&
    snapshot.preModelText !== snapshot.outputText
    ? snapshot.resultSessionId
    : null;
}

export function publicationLabel(stage: string | null): string | null {
  switch (stage) {
    case "smart_cleanup":
      return "Smart cleanup";
    case "smart_layout":
      return "Shape by type";
    case "layout_skipped":
      return "Layout skipped";
    case "as_heard":
      return "As heard";
    case "rules":
      return "Rules";
    case "punctuation_model":
      return "Punctuation";
    case "punctuation_rules":
      return "Punctuation rules";
    case "edited":
      return "Edited";
    default:
      return stage;
  }
}

export function guardReasonLabel(reason: string): string {
  if (reason.startsWith("layout:")) {
    const labels: Record<string, string> = {
      as_heard: "As heard kept unchanged",
      short_text: "Short message kept unchanged",
      message: "Message kept unchanged",
      unchanged: "Layout unchanged",
      low_confidence: "Layout uncertain; earlier text kept",
      words_changed: "Layout word change blocked",
      symbols_changed: "Layout symbol change blocked",
      sentence_case: "Layout sentence case rejected",
      dash: "Layout dash rejected",
      ellipsis: "Layout ellipsis rejected",
      markdown: "Layout markup rejected",
      newlines: "Layout spacing rejected",
      timeout: "Layout timed out",
      cleanup_stopped: "Layout stopped",
      cancelled: "Cancelled",
      busy: "Text model busy; earlier text kept",
      model_busy: "Text model busy; earlier text kept",
      model_unavailable: "Layout model unavailable",
      model_error: "Layout model unavailable",
      no_generation_model: "Layout model unavailable",
      incomplete_recognition: "Incomplete recognition kept unchanged",
      length_stop: "Incomplete layout kept out",
      incomplete_output: "Incomplete layout kept out",
      missing_logprobs: "Layout confidence unavailable",
      invalid_classification: "Layout classification unavailable",
      invalid_logprobs: "Layout confidence unavailable",
      ambiguous_classification: "Layout uncertain; earlier text kept",
      input_limit: "Text exceeds layout limit",
      context_limit: "Text exceeds layout limit",
      empty_output: "Empty layout kept out",
      empty_list_item: "Empty list item rejected",
      paragraphs_only: "Prose layout rejected",
    };
    return labels[reason.slice("layout:".length)] ?? "Layout skipped";
  }
  const labels: Record<string, string> = {
    number_changed: "Number change blocked",
    protected_span_changed: "Name or protected text change blocked",
    novel_content_words: "Added wording blocked",
    length_ratio: "Large text change blocked",
    reply_or_refusal: "Model reply blocked",
    empty_output: "Empty model result",
    timeout: "Cleanup timed out",
    length_stop: "Incomplete model result",
    incomplete_output: "Incomplete model result",
    model_error: "Cleanup unavailable",
    model_busy: "Cleanup busy",
    unsafe_deletion_or_reordering: "Word removal or reordering blocked",
    unsafe_formatting: "Formatting change blocked",
    cancelled: "Cancelled",
    cleanup_stopped: "Cleanup stopped",
    context_limit: "Text exceeds cleanup limit",
    input_limit: "Text exceeds cleanup limit",
    mode_not_clean: "Smart cleanup applies to Clean only",
  };
  return labels[reason] ?? "Cleanup guard";
}

export function historyVersions(record: DictationRecord) {
  const raw = { kind: "raw", title: "As recognized", text: record.raw };
  const published = {
    kind: "published",
    title:
      record.recognitionCompleteness === "incomplete"
        ? "Available text"
        : record.publicationStage === "smart_cleanup"
          ? "Cleaned"
          : record.publicationStage === "smart_layout"
            ? "Shaped"
            : "Finished text",
    text: record.published,
  };
  return record.preModelText === null
    ? [raw, published]
    : [
        {
          kind: "pre_model",
          title: "Before AI changes",
          text: record.preModelText,
        },
        published,
        raw,
      ];
}
