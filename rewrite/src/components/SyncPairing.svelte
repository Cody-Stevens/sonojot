<script lang="ts">
  import { onMount } from "svelte";
  import { z } from "zod";
  import { sync, syncAction, refreshSync } from "../lib/sync.svelte";

  let { trusted = false }: { trusted?: boolean } = $props();
  const requestsSchema = z.array(
    z.object({
      id: z.string(),
      device: z.string(),
      platform: z.string(),
      name: z.string().optional(),
      approver: z.string().nullable().optional(),
      state: z.string(),
      expires_at: z.number(),
    }),
  );
  const pollSchema = z.object({
    code: z
      .string()
      .regex(/^\d{6}$/)
      .optional(),
    unlocked: z.boolean().optional(),
    expired: z.boolean().optional(),
    denied: z.boolean().optional(),
  });
  let pair = $state("");
  let code = $state("");
  let device = $state("");
  let expires = 0;
  let phase = $state<"waiting" | "cancelled" | "expired" | "denied">("waiting");
  let error = $state("");
  let working = $state(false);
  let active = false;
  const digits = $derived(code ? `${code.slice(0, 3)} ${code.slice(3)}` : "");

  async function poll() {
    if (!active || working || sync.busy || error || phase !== "waiting") return;
    working = true;
    try {
      if (!pair) {
        if (trusted) {
          const requests = requestsSchema.parse(await syncAction("pair_list"));
          if (!active) return;
          const request = requests.find(
            (r) =>
              r.device !== sync.status?.device &&
              (r.state === "committed" ||
                (r.approver === sync.status?.device &&
                  ["accepted", "revealed", "ready"].includes(r.state))),
          );
          if (!request) return;
          device = request.name
            ? `${request.name} · ${request.platform}`
            : request.platform;
          await syncAction("pair_accept", { id: request.id });
          if (!active) return;
          pair = request.id;
          expires = request.expires_at * 1000;
        } else {
          const request = z
            .object({ id: z.string(), expiresAt: z.number() })
            .parse(await syncAction("pair_start"));
          if (!active) return;
          pair = request.id;
          expires = request.expiresAt * 1000;
        }
      }
      if (Date.now() >= expires) {
        code = "";
        pair = "";
        phase = trusted ? "waiting" : "expired";
        return;
      }
      const result = pollSchema.parse(
        await syncAction("pair_poll", { id: pair }),
      );
      if (!active) return;
      if (result.unlocked) {
        code = "";
        pair = "";
        await refreshSync();
      } else if (result.expired || result.denied) {
        code = "";
        pair = "";
        phase = trusted ? "waiting" : result.expired ? "expired" : "denied";
      } else if (result.code) code = result.code;
    } catch (e) {
      if (active) error = String(e);
    } finally {
      working = false;
    }
  }

  async function decide(approve: boolean) {
    if (working) return;
    working = true;
    error = "";
    try {
      if (pair)
        await syncAction(approve ? "pair_approve" : "pair_deny", {
          id: pair,
          ...(approve ? { confirmed: true } : {}),
        });
      if (!active) return;
      pair = "";
      code = "";
      phase = trusted ? "waiting" : "cancelled";
      await refreshSync();
    } catch (e) {
      if (active) error = String(e);
    } finally {
      working = false;
    }
  }
  function retry() {
    error = "";
    phase = "waiting";
    void poll();
  }
  onMount(() => {
    active = true;
    void poll();
    // Three seconds keeps both peers below the account's pairing read limit.
    const timer = setInterval(() => void poll(), 3000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  });
</script>

{#if !trusted || pair || error}
  <div
    class="approval"
    aria-label={trusted ? "New computer approval" : "Computer approval"}
  >
    <h2 data-text-surface="emboss">
      {trusted
        ? "A new computer wants to join sync"
        : "Approve on another device"}
    </h2>
    {#if trusted}<p data-text-surface="flat">{device}</p>{/if}
    {#if code}<output
        class="pair-code"
        aria-label="Six-digit code"
        data-text-surface="emboss">{digits}</output
      >{/if}
    <p data-text-surface="flat">
      {trusted
        ? "Check that the new computer shows the same six digits. Approve only if they match."
        : "On a computer where SonoJot sync is already set up, open Settings → Account and sync and approve this computer. Check that it shows the same six digits."}
    </p>
    {#if error}
      <p role="alert">{error}</p>
      <button class="button" disabled={working} onclick={retry}
        >Try again</button
      >
    {:else if phase === "waiting" && (!trusted || !code)}
      <p class="waiting" role="status">
        <span class="spinner" aria-hidden="true"></span>{!trusted && code
          ? "Waiting for approval…"
          : "Preparing six digits…"}
      </p>
    {:else if phase !== "waiting"}
      <p role="status">
        {phase === "expired"
          ? "This code expired."
          : phase === "denied"
            ? "This request was denied."
            : "Approval cancelled."}
      </p>
      <button class="button" disabled={working} onclick={retry}
        >Get a new code</button
      >
    {/if}
    {#if phase === "waiting"}
      <div class="actions">
        {#if trusted}<button
            class="button"
            disabled={working || sync.busy || !code || !!error}
            onclick={() => decide(true)}>Approve</button
          >{/if}
        <button
          class="button"
          disabled={working || sync.busy}
          onclick={() => decide(false)}>{trusted ? "Deny" : "Cancel"}</button
        >
      </div>
    {/if}
  </div>
{/if}

<style>
  .approval {
    display: grid;
    gap: 18px;
  }
  h2,
  p {
    margin: 0;
  }
  .pair-code {
    font-size: 2.5rem;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.12em;
    line-height: 1.2;
    user-select: text;
  }
  .actions,
  .waiting {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--line);
    border-top-color: var(--ink);
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }
</style>
