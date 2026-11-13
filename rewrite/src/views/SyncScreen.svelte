<script lang="ts">
  import { onMount } from "svelte";
  import Sheet from "../components/quiet/Sheet.svelte";
  import { listen } from "@tauri-apps/api/event";
  import { z } from "zod";
  import { save } from "@tauri-apps/plugin-dialog";
  import { sync, runSync, refreshSync } from "../lib/sync.svelte";
  const devicesSchema = z.array(
    z.object({
      id: z.string(),
      platform: z.string(),
      revoked_at: z.number().nullable(),
    }),
  );
  const pairsSchema = z.array(
    z.object({
      id: z.string(),
      device: z.string(),
      platform: z.string(),
      location: z.string(),
      state: z.string(),
      created_at: z.number().optional(),
    }),
  );
  let devices = $state<z.infer<typeof devicesSchema>>([]);
  let requests = $state<z.infer<typeof pairsSchema>>([]);
  let recovery = $state("");
  let entered = $state("");
  let saved = $state(false);
  let pair = $state("");
  let code = $state("");
  let approver = $state(false);
  let confirm = $state<
    | "setup"
    | "reset_request"
    | "reset"
    | "recovery_new"
    | "account_delete"
    | "remove"
    | null
  >(null);
  let removing = $state("");
  let signoutOnly = $state(false);
  let clock = $state(Date.now());
  const resetReady = $derived(
    !!sync.status?.pendingReset && clock >= sync.status.pendingReset * 1000,
  );
  let server = $state("");
  async function reload() {
    await refreshSync();
    if (sync.status?.signedIn) {
      await runSync("remote_status");
      const d = await runSync("devices");
      if (d) devices = devicesSchema.parse(d);
      if (sync.status.unlocked) {
        const p = await runSync("pair_list");
        if (p) requests = pairsSchema.parse(p);
      }
    }
  }
  onMount(() => {
    const timer = setInterval(() => {
      clock = Date.now();
    }, 1000);
    void reload();
    let dispose: undefined | (() => void);
    let active = true;
    void listen("sync-changed", () => {
      void refreshSync();
    }).then((fn) => {
      if (active) dispose = fn;
      else fn();
    });
    return () => {
      clearInterval(timer);
      active = false;
      dispose?.();
      recovery = "";
      entered = "";
    };
  });
  async function start(provider: string) {
    await runSync("signin", { provider, ...(server ? { server } : {}) });
  }
  async function confirmed() {
    const op = confirm;
    confirm = null;
    if (!op) return;
    const result = await runSync(op, {
      confirmed: true,
      ...(op === "remove"
        ? {
            device: removing,
            rekey: removing !== sync.status?.device && !signoutOnly,
          }
        : {}),
    });
    if (
      op === "setup" ||
      op === "reset" ||
      op === "recovery_new" ||
      (op === "remove" && removing !== sync.status?.device && !signoutOnly)
    ) {
      if (result) {
        const r = await runSync("recovery_show", { confirmed: true });
        if (r) recovery = z.object({ code: z.string() }).parse(r).code;
      }
    }
    await reload();
  }
  async function pairStart() {
    const r = await runSync("pair_start");
    if (r) {
      pair = z.object({ id: z.string() }).parse(r).id;
      approver = false;
      code = "";
    }
  }
  async function accept(id: string) {
    const r = await runSync("pair_accept", { id });
    if (r) {
      pair = id;
      approver = true;
      code = "";
    }
  }
  async function poll() {
    const r = await runSync("pair_poll", { id: pair });
    if (!r) return;
    const state = z
      .object({ code: z.string().optional(), unlocked: z.boolean().optional() })
      .parse(r);
    if (state.code) code = state.code;
    if (state.unlocked) {
      pair = "";
      await reload();
    }
  }
  async function saveCode() {
    const path = await save({
      defaultPath: "SonoJot-recovery.txt",
      filters: [{ name: "Text", extensions: ["txt"] }],
    });
    if (path) await runSync("recovery_download", { path, confirmed: true });
  }
</script>

<section class="stock paper-card sync-sheet" aria-busy={sync.busy}>
  {#if !sync.status?.signedIn}
    <p>
      Keep your words on your devices. Vocabulary sync is free and end-to-end
      encrypted.
    </p>
    <div class="sync-actions">
      <button
        class="button"
        disabled={sync.busy}
        onclick={() => start("authkit")}>Continue with WorkOS AuthKit</button
      >
    </div>
    <details>
      <summary>Self-hosted server</summary><label
        >Server address<input
          type="url"
          class="well"
          placeholder="https://sync.example.com"
          bind:value={server}
        /></label
      >
    </details>
  {:else}
    <strong data-text-surface="emboss">{sync.status.email}</strong>
    {#if sync.status.pendingReset}
      <p role="alert">
        A reset was requested. It can erase encrypted sync after {new Date(
          sync.status.pendingReset * 1000,
        ).toLocaleString()}.
      </p>
      {#if sync.status.unlocked}<button
          class="button"
          onclick={() => runSync("reset_cancel")}>Cancel reset</button
        >
      {:else}
        <p>
          Available {new Date(
            sync.status.pendingReset * 1000,
          ).toLocaleString()}. This request expires seven days later.
        </p>
        <button
          class="button"
          disabled={sync.busy || !resetReady}
          onclick={() => (confirm = "reset")}
          >Complete reset after 72 hours</button
        >
      {/if}
    {/if}
    {#if sync.status.resetCancelled}<p role="status">
        SonoJot cancelled a sync reset because this device can unlock your
        words. If this was not you, secure your sign-in account and re-key.
      </p>{/if}
    {#if sync.status.notice}
      <p role="alert">{sync.status.notice}</p>
      <button
        class="button"
        onclick={() => runSync("journal_ack", { confirmed: true })}
        >I reviewed my local words</button
      >
    {/if}
    <details>
      <summary>Sign in again or update contact email</summary>
      <p>
        WorkOS manages Google, Apple, email codes and passkeys. Confirm contact
        email changes here on an unlocked device; reset notices keep going to
        the previous address until then.
      </p>
      <button
        class="button"
        disabled={sync.busy}
        onclick={() => start("authkit")}>Continue with WorkOS AuthKit</button
      >
      <button
        class="button"
        disabled={sync.busy || !sync.status.unlocked}
        onclick={() => runSync("signin", { provider: "authkit", link: true })}
        >Confirm contact email or link WorkOS account</button
      >
    </details>
    {#if !sync.status.unlocked}
      <h2 data-text-surface="emboss">Unlock your words</h2>
      <p>
        Approve this computer on another device, or enter your recovery code.
        Signing in alone cannot unlock your words.
      </p>
      <button class="button" disabled={sync.busy} onclick={pairStart}
        >Approve on another device</button
      >
      <form
        onsubmit={async (e) => {
          e.preventDefault();
          await runSync("recovery_use", { code: entered });
          entered = "";
          await reload();
        }}
      >
        <label
          >Recovery code<input
            class="well"
            type="password"
            autocomplete="off"
            bind:value={entered}
          /></label
        ><button class="button" disabled={sync.busy || !entered}>Unlock</button>
      </form>
      <button
        class="button"
        disabled={sync.busy}
        onclick={() => (confirm = "reset_request")}
        >I lost every device and my recovery code</button
      >
      <button
        class="button"
        disabled={sync.busy}
        onclick={() => (confirm = "setup")}
        >Set up sync for the first time</button
      >
    {:else}
      <p class="field-help" data-text-surface="flat">
        {sync.status.lastSync
          ? `Last synced ${new Date(sync.status.lastSync * 1000).toLocaleString()}`
          : "Ready to sync vocabulary"}
      </p>
      <div class="sync-actions">
        <button
          class="button"
          disabled={sync.busy}
          onclick={() => runSync("now")}>Sync now</button
        ><button class="button" disabled={sync.busy} onclick={reload}
          >Refresh devices</button
        ><button
          class="button"
          disabled={sync.busy}
          onclick={() => (confirm = "recovery_new")}>New recovery code</button
        >
      </div>
      {#if sync.status.recoveryPending && !recovery}<button
          class="button"
          onclick={async () => {
            const r = await runSync("recovery_show", { confirmed: true });
            if (r) recovery = z.object({ code: z.string() }).parse(r).code;
          }}>Save recovery code</button
        >{/if}
      {#each requests.filter((r) => r.device !== sync.status?.device) as request}<div
          class="sync-row"
        >
          <span>{request.platform} · {request.location}</span><button
            class="button"
            disabled={sync.busy}
            onclick={() => accept(request.id)}>Review new device</button
          >
        </div>{/each}
    {/if}
    {#each devices.filter((d) => d.revoked_at === null) as device}<div
        class="sync-row"
      >
        <span
          >{device.platform}{device.id === sync.status.device
            ? " · this device"
            : ""}</span
        ><button
          class="button"
          disabled={sync.busy}
          onclick={() => {
            removing = device.id;
            signoutOnly = false;
            confirm = "remove";
          }}>Remove</button
        >
      </div>{/each}
    <div class="sync-actions">
      <button
        class="button"
        disabled={sync.busy}
        onclick={() => runSync("signout")}>Sign out</button
      ><button
        class="button"
        disabled={sync.busy}
        onclick={async () => {
          const path = await save({ defaultPath: "SonoJot-account.json" });
          if (path) await runSync("account_export_file", { path });
        }}>Download account data</button
      ><button
        class="button"
        disabled={sync.busy}
        onclick={() => (confirm = "account_delete")}>Delete account</button
      >
    </div>
  {/if}
</section>
{#if pair}<section class="stock paper-card sync-sheet">
    <h2 data-text-surface="emboss">
      {approver ? "Approve this device" : "Approve on your other device"}
    </h2>
    <p>Compare all six digits on both screens.</p>
    {#if code}<output class="pair-code">{code}</output>{/if}
    <div class="sync-actions">
      <button class="button" disabled={sync.busy} onclick={poll}
        >Check pairing</button
      >{#if code}<button
          class="button"
          disabled={sync.busy}
          onclick={async () => {
            if (
              await runSync(approver ? "pair_approve" : "pair_confirm", {
                id: pair,
                confirmed: true,
              })
            ) {
              pair = "";
              await reload();
            }
          }}>They match</button
        >{/if}<button
        class="button"
        disabled={sync.busy}
        onclick={async () => {
          await runSync("pair_deny", { id: pair });
          pair = "";
        }}>Deny</button
      >
    </div>
  </section>{/if}
{#if recovery}<section class="stock paper-card sync-sheet">
    <h2 data-text-surface="emboss">Save your recovery code</h2>
    <p>
      If you lose every device and this code, no one can recover your synced
      words.
    </p>
    <output class="recovery-code">{recovery}</output>
    <div class="sync-actions">
      <button
        class="button"
        onclick={() => runSync("recovery_copy", { confirmed: true })}
        >Copy for password manager</button
      ><button class="button" onclick={saveCode}>Download</button><button
        class="button"
        onclick={() => runSync("recovery_print", { confirmed: true })}
        >Print</button
      >
    </div>
    <label
      ><input type="checkbox" bind:checked={saved} /> I saved my recovery code</label
    ><button
      class="button"
      disabled={!saved || sync.busy}
      onclick={async () => {
        if (await runSync("recovery_ack", { confirmed: true })) {
          recovery = "";
          saved = false;
        }
      }}>Done</button
    >
  </section>{/if}
{#if confirm}<Sheet
    title="Confirm account change"
    onclose={() => (confirm = null)}
    ><div class="sync-sheet">
      <h2 data-text-surface="emboss">
        {confirm === "setup"
          ? "Set up encrypted sync?"
          : confirm === "reset_request"
            ? "Request a reset in 72 hours?"
            : confirm === "reset"
              ? "Start over with new encryption keys?"
              : confirm === "recovery_new"
                ? "Replace your recovery code?"
                : confirm === "remove"
                  ? "Remove this device?"
                  : "Delete your sync account?"}
      </h2>
      <p>
        {confirm === "setup"
          ? "Your device creates the encryption key. Save the recovery code on the next screen."
          : confirm === "recovery_new"
            ? "The previous code will stop working. Save the new code before closing this screen."
            : confirm === "remove"
              ? "This revokes its sessions. Copies already on that device remain."
              : confirm === "reset_request"
                ? "Your encrypted words stay in place during the waiting period."
                : "Synced data and account access will be deleted. Local vocabulary stays on this computer."}
      </p>
      {#if confirm === "remove" && removing !== sync.status?.device && sync.status?.unlocked}<label
          ><input type="checkbox" bind:checked={signoutOnly} />Sign out only, no
          re-key</label
        >
        <p>
          {signoutOnly
            ? "This device keeps its keys and can regain access by signing in again."
            : "Re-key protects future writes and creates a new recovery code; other devices must pair again."}
        </p>
      {/if}
      {#if confirm === "reset_request"}<p>
          Any unlocked device cancels this request automatically when it syncs.
          The request expires seven days after the wait. An email notice is
          queued when email delivery is enabled.
        </p>{/if}
      {#if confirm === "reset"}<p>
          No one can recover the old encrypted copy. This deletes it and removes
          every other device. Your local words stay here.
        </p>{/if}
      <div class="sync-actions">
        <button
          class="button"
          disabled={sync.busy || (confirm === "reset" && !resetReady)}
          onclick={confirmed}>Confirm</button
        ><button class="button" onclick={() => (confirm = null)}>Cancel</button>
      </div>
    </div></Sheet
  >{/if}
{#if sync.busy}<p role="status" class="field-help">Connecting…</p>{/if}
{#if sync.error}<p role="alert">{sync.error}</p>
  <button class="button" disabled={sync.busy} onclick={reload}>Try again</button
  >{/if}

<style>
  .sync-sheet {
    padding: 27px;
    margin-bottom: 18px;
    display: grid;
    gap: 18px;
  }
  .sync-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 9px;
  }
  .sync-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 18px;
  }
  .sync-sheet input[type="checkbox"] {
    appearance: auto;
    width: 18px;
    height: 18px;
    min-height: 0;
    padding: 0;
    margin: 0;
    border: initial;
    box-shadow: none;
    accent-color: var(--ink);
  }
  .sync-sheet label:has(input[type="checkbox"]) {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .sync-sheet p {
    margin: 0;
  }
  .sync-sheet label {
    display: grid;
    gap: 9px;
  }
  .sync-sheet form {
    display: grid;
    gap: 9px;
  }
  .pair-code {
    font-size: 2rem;
    letter-spacing: 0.2em;
    font-variant-numeric: tabular-nums;
  }
  .recovery-code {
    overflow-wrap: anywhere;
    user-select: text;
  }
  .sync-sheet details input {
    width: 100%;
    margin-top: 9px;
  }
</style>
