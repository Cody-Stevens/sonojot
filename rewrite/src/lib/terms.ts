import type { Settings } from "./protocol";

/** Increase when bundled terms change, requiring renewed acceptance. */
export const TERMS_VERSION = 1;

type TermsAcceptance = Pick<
  Settings,
  "termsAcceptedVersion" | "termsAcceptedAt"
>;

export function needsTermsAcceptance(settings: TermsAcceptance): boolean {
  return (
    settings.termsAcceptedVersion < TERMS_VERSION ||
    !settings.termsAcceptedAt?.trim()
  );
}

export function termsAcceptance(at = new Date()): TermsAcceptance {
  return {
    termsAcceptedVersion: TERMS_VERSION,
    termsAcceptedAt: at.toISOString(),
  };
}
