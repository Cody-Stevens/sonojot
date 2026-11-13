# SonoJot sync staging

Stage 1 synchronizes vocabulary only. The Worker stores identity metadata, hashes
of sessions, wrapped keys and padded ciphertext. It has no recovery key and no
server-side content decryption. Provider identity never derives an encryption key.

Use Node 22+ and the lockfile. Dependency lifecycle scripts are disabled by the
`--ignore-scripts` install option.

```text
npm ci --ignore-scripts
npm run check
npm test
npm run test:e2e
npm run build
node scripts/smoke.mjs
```

The E2E harness starts isolated Miniflare, signs two Rust clients in through mocked WorkOS AuthKit and passes their synthetic session
credentials in process environment only, exercises propagation/conflict/re-key,
inspects R2 for ciphertext and disposes the runtime. It has a five-minute process
limit. Synthetic staging sign-in requires the configured test secret and is unavailable outside staging. The page smoke requires
the app's existing Playwright dependency and a completed `web/` build; it uses
intercepted in-memory requests and closes its browser and Miniflare.

Deployment is restricted by `scripts/staging-guard.mjs`. From this directory:

```text
npm run migrate:staging
npm run deploy:staging
```

The public `wrangler.jsonc` has placeholders. The wrapper reads non-secret
operator settings from `SONOJOT_SYNC_CONFIG` (a JSON file path), or from
`../scripts/sync-staging.json` in the private checkout. It fills only `origin`,
`accountId`, `databaseId` and `workosClientId` into a temporary config, validates
the staging restrictions, runs Wrangler with that config and removes it on exit.
`npm run build` performs the same steps with `--dry-run`; `node
scripts/staging-guard.mjs check` validates without invoking Wrangler.

A self-hosted staging deployment uses its own values in this shape, outside the
public source tree:

```json
{
  "origin": "https://sonojot-sync-staging.example.workers.dev",
  "accountId": "YOUR_CLOUDFLARE_ACCOUNT_ID",
  "databaseId": "YOUR_STAGING_DATABASE_ID",
  "workosClientId": "YOUR_PUBLIC_AUTHKIT_CLIENT_ID"
}
```

Replace the example origin with your staging Worker's HTTPS origin. Build the
prerelease or Sync Test app with `SONOJOT_SYNC_ORIGIN` set to the same origin;
without it the app keeps sync disabled. Stable builds always disable sync. The
app accepts only the `sonojot-sync-staging.<account>.workers.dev` hostname form.
Keep credentials in managed Worker secrets, never in this JSON file. Bare
Wrangler commands use the placeholder config; use the wrappers for staging.

All Worker/D1/R2 names end in `-staging`. There are no custom routes or production
environment in this configuration. Provider secrets and email remain disabled.
Do not use `.dev.vars` for actual
credentials, commit keys, or pass codes in command arguments.

The 03:17 UTC cron expires unused accounts, sessions, challenges and old bodies;
exports D1 metadata and ciphertext to private R2; and expires snapshots after
30 days. A manifest written last marks a complete snapshot. Session credentials
are never restored. Account-erasure markers last 35 days and override snapshots.
All tables are materialized together in a D1 transaction, then exported in 250-row pages with a schema-2 manifest. There is no 1,000-row table limit. Changes older than 30 days are pruned; stale cursors receive a full-resync signal.

`src/restore.ts` is an operator library, never an HTTP route. Restore into an
empty staging database with all migrations applied and a private destination
bucket. Supply the source backup bucket, complete snapshot prefix and any
additional erasure exclusions. The function also reads retained erasure markers.
Validate ciphertext, byte accounting, revision/epoch continuity and fresh sign-in
before directing any clients there. Never overwrite a live account database or
lower a device's rollback checkpoint to make an old restore appear current.

`test/worker.test.ts` contains the repeatable restore drill: snapshot ciphertext,
clear the isolated database and content bucket, restore, sign in afresh and compare
the envelope. A second case deletes an account after the snapshot and verifies
that it is not restored. The live staging cron's first export still needs a
post-run check; local drill evidence is not a claim of a live restore.

Health uses aggregate 5xx rate, maintenance freshness and storage-growth checks.
Operational counts expire after seven days. Application request logging is off;
emails, bodies, credentials, callback URLs and raw exceptions must stay out of
logs. Enrollment in the shared monitor and PatchBell awaits separate approval.
There are no billing routes, payments, meeting sync or audio sync in this stage.

Security protocol v2 requires a user-confirmed SAS on both devices and Auth HPKE
from the confirmed approver key. AK-derived proof protects mutations. Initial wraps
and their proof verifier are installed atomically at `/v1/bootstrap`; existing
pre-v2 staging wraps cannot be claimed by a provider session. Such accounts must
use the 72-hour reset. Reset requests are visible in `/v1/poll`, cancellable by
an unlocked device, and queued for email only when `MAIL_ENABLED` permits it.
Keyless web sessions cannot remove devices or delete encrypted data.

Run `node scripts/generate-vectors.mjs` to regenerate public synthetic envelope,
Auth HPKE, recovery, pairing and server-auth fixtures in
`rewrite/src-tauri/tests/fixtures/`. No live keys are inputs.
An idle `/v1/poll` performs no writes, including rate-limit and monitor counters.
Session renewal uses a separate daily heartbeat. The expanded restore drill uses
1,105 change records and verifies pruning/full-resync after restoration.


AuthKit uses browser-bound PKCE and signed lifecycle events. Migration 0006
removes the retired email-code table and adds WorkOS session mappings/tombstones.
No Google/Apple ID-token validation or application email-code sender remains.
Current SonoJot contact email changes require a fresh WorkOS login plus AK proof;
webhooks never overwrite it. Live AuthKit stays unavailable until the public
client ID and two managed secrets are set. Security-notice mail remains disabled.

Synthetic test identities require both the staging environment and its managed test secret. The deployment wrappers always select staging; the base config is deliberately unconfigured.
