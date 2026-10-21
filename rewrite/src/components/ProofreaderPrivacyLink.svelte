<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { OPENROUTER_PRIVACY_URL } from "../lib/proofreader";

  function open(event: MouseEvent): void {
    if (!app.native) return;
    event.preventDefault();
    if (event.button > 1) return;
    void app.perform("openrouter-privacy", () =>
      call("open_openrouter_privacy"),
    );
  }
</script>

<p class="field-help" data-text-surface="flat" role="status">
  Your OpenRouter account blocks this training model. Earlier text was kept.
  <a href={OPENROUTER_PRIVACY_URL} onclick={open} onauxclick={open}
    >Open account privacy settings</a
  >
</p>
