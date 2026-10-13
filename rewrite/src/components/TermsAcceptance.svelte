<script lang="ts">
  import LegalLinks from "./LegalLinks.svelte";
  import TermsText from "./TermsText.svelte";
  import { TERMS_VERSION } from "../lib/terms";
  let {
    agreed = $bindable(false),
    disabled = false,
  }: { agreed?: boolean; disabled?: boolean } = $props();
</script>

<p class="body-copy" data-text-surface="flat">
  Version {TERMS_VERSION}. Your agreement and its date stay on this device.
  Optional downloads have separate terms.
</p>
<TermsText />
<LegalLinks />
<label class="agreement">
  <input type="checkbox" bind:checked={agreed} {disabled} />
  <span>I agree to the SonoJot Terms of Use</span>
</label>

<style>
  .agreement {
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
