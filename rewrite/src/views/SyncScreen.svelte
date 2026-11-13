<script lang="ts">
  import MeetingSyncNotice from "../components/quiet/MeetingSyncNotice.svelte";
  import { onMount, untrack } from "svelte";
  import SyncPairing from "../components/SyncPairing.svelte";
  import MeetingSyncStorage from "../components/quiet/MeetingSyncStorage.svelte";
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
  let devices = $state<z.infer<typeof devicesSchema>>([]);
  let alternatives = $state(false);
  let useRecovery = $state(false);
  let accountOptions = $state(false);
  let recovery = $state("");
  let entered = $state("");
  let saved = $state(false);
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
  let reloading = false;
  const accountState = $derived(
    `${sync.status?.signedIn}:${sync.status?.device}:${sync.status?.unlocked}`,
  );
  $effect(() => {
    accountState;
    untrack(() => {
      alternatives = false;
      useRecovery = false;
      accountOptions = false;
      entered = "";
      if (!sync.status?.signedIn) recovery = "";
      void reload();
    });
  });
  async function reload() {
    if (reloading) return;
    reloading = true;
    try {
      await refreshSync();
      if (sync.status?.signedIn) {
        await runSync("remote_status");
        if (sync.status?.unlocked) {
          const d = await runSync("devices");
          if (d) devices = devicesSchema.parse(d);
        } else devices = [];
      } else devices = [];
    } finally {
      reloading = false;
    }
  }
  onMount(() => {
    const timer = setInterval(() => {
      clock = Date.now();
    }, 1000);
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
  async function start(intent = "sign-in") {
    await runSync("signin", {
      intent,
      provider: "email",
      ...(server ? { server } : {}),
    });
  }
  async function confirmed() {
    const op = confirm;
    confirm = null;
    if (!op) return;
    const rekey =
      op === "remove" && removing !== sync.status?.device && !signoutOnly;
    const result = await runSync(op, {
      confirmed: true,
      ...(op === "remove"
        ? {
            device: removing,
            rekey,
          }
        : {}),
    });
    if (op === "setup" || op === "reset" || op === "recovery_new" || rekey) {
      if (result) {
        const r = await runSync("recovery_show", { confirmed: true });
        if (r) recovery = z.object({ code: z.string() }).parse(r).code;
      }
    }
    await reload();
  }
  async function saveCode() {
    const path = await save({
      defaultPath: "SonoJot-recovery.txt",
      filters: [{ name: "Text", extensions: ["txt"] }],
    });
    if (path) await runSync("recovery_download", { path, confirmed: true });
  }
</script>

{#if !recovery}
  <section class="stock paper-card sync-sheet">
    {#if !sync.status?.signedIn}
      <p>
        Keep your words on your devices. Vocabulary sync is free and end-to-end
        encrypted.
      </p>
      <div class="sync-actions">
        <button
          class="button"
          disabled={sync.busy}
          onclick={() => start("sign-up")}>Set up sync</button
        >
        <button class="button" disabled={sync.busy} onclick={() => start()}
          >Add this computer</button
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
    {:else if !sync.status.unlocked}
      {#if sync.status.initialized === false}
        <h2 data-text-surface="emboss">Set up sync</h2>
        <p>
          This computer creates your encryption key. Save the recovery code
          next, and you’re set.
        </p>
        <button
          class="button"
          disabled={sync.busy}
          onclick={async () => {
            confirm = "setup";
            await confirmed();
          }}>Create encryption key</button
        >
      {:else}
        <SyncPairing />
        <button
          class="text-action"
          aria-expanded={alternatives}
          onclick={() => (alternatives = !alternatives)}
          >Can't approve from another computer?</button
        >
        {#if alternatives}
          <div class="sync-options">
            <button
              class="button"
              aria-expanded={useRecovery}
              onclick={() => (useRecovery = !useRecovery)}
              >Use recovery code</button
            >
            {#if useRecovery}
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
                >
                <button class="button" disabled={sync.busy || !entered}
                  >Unlock</button
                >
              </form>
            {/if}
            <button
              class="text-action"
              disabled={sync.busy}
              onclick={() => (confirm = "reset_request")}
              >I lost my devices and recovery code</button
            >
            {#if sync.status.pendingReset}
              <p role="status">
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
          </div>
        {/if}
      {/if}
      <button
        class="text-action"
        disabled={sync.busy}
        onclick={() => runSync("signout")}>Sign out</button
      >
    {:else}
      <SyncPairing trusted />
      <strong data-text-surface="emboss">{sync.status.email}</strong>
      <p data-text-surface="flat">
        Words and meetings · {devices.filter((d) => d.revoked_at === null)
          .length}
        {devices.filter((d) => d.revoked_at === null).length === 1
          ? "device"
          : "devices"}
      </p>
      <p class="field-help" data-text-surface="flat">
        {sync.status.lastSync
          ? `Last synced ${new Date(sync.status.lastSync * 1000).toLocaleString()}`
          : "Ready to sync vocabulary and meetings"}
      </p>
      {#if sync.status.pendingReset}
        <p role="alert">
          A reset was requested. It can erase encrypted sync after {new Date(
            sync.status.pendingReset * 1000,
          ).toLocaleString()}.
        </p>
        <button
          class="button"
          disabled={sync.busy}
          onclick={() => runSync("reset_cancel")}>Cancel reset</button
        >
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
      <button class="button" disabled={sync.busy} onclick={() => runSync("now")}
        >Sync now</button
      >
      <MeetingSyncStorage />
      <button
        class="text-action"
        aria-expanded={accountOptions}
        onclick={() => (accountOptions = !accountOptions)}
        >Account options</button
      >
      {#if accountOptions}
        <div class="sync-options">
          <h2 data-text-surface="emboss">Devices</h2>
          {#each devices.filter((d) => d.revoked_at === null) as device}
            <div class="sync-row">
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
            </div>
          {/each}
          <button class="button" disabled={sync.busy} onclick={reload}
            >Refresh devices</button
          >
          <button
            class="button"
            disabled={sync.busy}
            onclick={() => (confirm = "recovery_new")}>New recovery code</button
          >
          {#if sync.status.recoveryPending}
            <button
              class="button"
              disabled={sync.busy}
              onclick={async () => {
                const r = await runSync("recovery_show", { confirmed: true });
                if (r) recovery = z.object({ code: z.string() }).parse(r).code;
              }}>Save recovery code</button
            >
          {/if}
          <details>
            <summary>Sign in again or update contact email</summary>
            <p>
              Sign in with an email code. Confirm contact email changes here on
              an unlocked computer; reset notices keep going to the previous
              address until then.
            </p>
            <button class="button" disabled={sync.busy} onclick={() => start()}
              >Sign in securely</button
            >
            <button
              class="button"
              disabled={sync.busy}
              onclick={() =>
                runSync("signin", { provider: "email", link: true })}
              >Confirm contact email</button
            >
          </details>
          <div class="sync-actions">
            <button
              class="text-action"
              disabled={sync.busy}
              onclick={() => runSync("signout")}>Sign out</button
            >
            <button
              class="button"
              disabled={sync.busy}
              onclick={async () => {
                const path = await save({
                  defaultPath: "SonoJot-account.json",
                });
                if (path) await runSync("account_export_file", { path });
              }}>Download account data</button
            >
            <button
              class="text-action"
              disabled={sync.busy}
              onclick={() => (confirm = "account_delete")}
              >Delete account</button
            >
          </div>
        </div>
      {/if}
    {/if}
  </section>
{/if}
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
            : "Your other computers will pick up the new key on their next sync. Save the new recovery code; the previous code stops working."}
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
          onclick={confirmed}
          >{confirm === "remove" ? "Remove device" : "Confirm"}</button
        ><button class="button" onclick={() => (confirm = null)}>Cancel</button>
      </div>
    </div></Sheet
  >{/if}
{#if sync.status?.unlocked}<MeetingSyncNotice />{/if}
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
  .sync-options {
    display: grid;
    gap: 18px;
  }
  .text-action {
    justify-self: start;
    border: 0;
    background: none;
    box-shadow: none;
    padding: 0;
    color: var(--ink);
    text-decoration: underline;
    text-underline-offset: 3px;
    font-size: 0.875rem;
  }
  .text-action:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 4px;
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
