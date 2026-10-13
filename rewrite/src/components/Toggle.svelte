<script lang="ts">
  let {
    label,
    checked,
    disabled = false,
    onchange,
  }: {
    label: string;
    checked: boolean;
    disabled?: boolean;
    onchange: (value: boolean) => void;
  } = $props();
</script>

<button
  type="button"
  class="switch"
  role="switch"
  aria-label={label}
  title={label}
  aria-checked={checked}
  {disabled}
  onclick={() => onchange(!checked)}><span aria-hidden="true"></span></button
>
