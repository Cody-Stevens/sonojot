// Which languages can be transcribed. This module has no imports, so a host can
// check eligibility (`@sonojot/transcribe/language`) before the user opts in,
// without bundling the worker, onnxruntime or the text data.

/** Primary language subtags that can be transcribed. */
export const SUPPORTED_LANGUAGES: readonly string[] = ["en"];

/** True when the BCP 47 tag can be transcribed. No tag means the default, English. */
export function supportsLanguage(language: string | undefined): boolean {
  if (!language) return true;
  const primary = /^([a-z]{2,3})(?:-|$)/i.exec(language)?.[1]?.toLowerCase();
  return primary !== undefined && SUPPORTED_LANGUAGES.includes(primary);
}

export class UnsupportedLanguageError extends Error {
  readonly language: string;

  constructor(language: string) {
    super(`Transcription in "${language}" is not available yet; only English is supported.`);
    this.name = "UnsupportedLanguageError";
    this.language = language;
  }
}
