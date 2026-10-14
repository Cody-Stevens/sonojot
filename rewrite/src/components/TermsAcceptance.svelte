<script lang="ts">
  import LegalLinks from "./LegalLinks.svelte";
  import TermsText from "./TermsText.svelte";
  import { TERMS_VERSION } from "../lib/terms";
  let {
    agreed = $bindable(false),
    disabled = false,
  }: { agreed?: boolean; disabled?: boolean } = $props();
</script>

<!-- Only the Terms panel scrolls. It shrinks to fit, so the links and the
     checkbox below it are always on screen. -->
<div class="terms-acceptance">
  <p class="body-copy" data-text-surface="flat">
    Version {TERMS_VERSION}. Your agreement and its date stay on this device.
    Optional downloads have separate terms.
  </p>
  <TermsText fill />
  <LegalLinks />
  <label class="agreement">
    <input type="checkbox" bind:checked={agreed} {disabled} />
    <span>I agree to the SonoJot Terms of Use</span>
  </label>
</div>

<style>
  .terms-acceptance {
    flex: 1 1 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .terms-acceptance > :global(.body-copy) {
    flex: none;
    margin: 0 0 18px;
  }
  .terms-acceptance > :global(.legal-links) {
    flex: none;
    margin: 18px 0 9px;
  }
  .agreement {
    flex: none;
    display: flex;
    align-items: flex-start;
    gap: 12px;
    line-height: 1.5;
    cursor: pointer;
  }
  input {
    flex: none;
    width: 18px;
    height: 18px;
    margin-top: 4px;
    accent-color: var(--ink);
  }
</style>
