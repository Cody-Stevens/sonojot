import { app } from "./app.svelte";
import { needsTermsAcceptance, termsAcceptance } from "./terms";

export async function acceptTerms(): Promise<void> {
  if (!needsTermsAcceptance(app.snapshot.settings)) return;
  await app.saveSettings(termsAcceptance());
  // saveSettings reports failures in app.error instead of rejecting. Never
  // advance on a checkbox alone when the persisted snapshot is still pending.
  if (needsTermsAcceptance(app.snapshot.settings))
    throw new Error(
      app.error || "Your agreement could not be saved. Try again.",
    );
}
