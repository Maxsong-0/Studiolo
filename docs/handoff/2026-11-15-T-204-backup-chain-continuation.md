# T-204 backup chain — continuation handoff (2026-11-15)

> **Superseded on 2026-11-15 (evening).** Sub-units 3–7 below were redone by the main session and are in the
> repository; do **not** redo them from this brief. Read "Status after the redo" at the end of this file first.

Written by the main session (Claude, DGX) for a cloud agent that cannot read the
project vault. The vault stays authoritative; this file is a snapshot.

## Goal

Finish the remaining code of the full Rust backup chain (task T-204) so that, in
tests, a sealed checkpoint can be backed up by the separate worker process to a
local restic repository, restored into a brand-new root, and every database
reopens and verifies. No production action of any kind.

## Base and branch

- Base branch: `claude/project-code-completion-40e514` (contains the whole
  integration line plus the baseline fixes of 2026-11-15).
- Work on your own new branch `cursor/t204-backup-chain`, created from that base.
- Push your branch. Do **not** open a PR, do **not** push to `main` or to the
  base branch, do **not** merge anything.

## Decided — do not reopen

Everything in these two files is settled design; read them first, in full:

1. `docs/handoff/2026-11-13-T-204-策略checkpoint接线实施记录.md` — the running
   implementation record (Chinese). Sections from 「原链持久准入」 to the end
   describe the current state precisely.
2. `docs/handoff/ADR-031-策略checkpoint使用SQLite写屏障.md`.

Key points you must keep:

- The worker is a separate process with a separate system UID. It only consumes
  an `ApprovedJob` obtained through the fixed producer socket; no caller-supplied
  key / reference / path may substitute for it.
- Provenance is a fact recorded by the trusted side before launch
  (`ArtifactProvenance`, Control schema 4 `artifact_policies`). A `RunnerReport`
  never carries authorization. Old attempts with no record never get `Full`
  by default; unknown sources fail closed. Never classify by file name.
- Credentials come only from the fixed systemd credentials directory described
  in the record. Never read them from environment or request-supplied paths.
- Checks that fail closed (mlockall / `RLIMIT_MEMLOCK`, same-device checks,
  identity and permission checks) stay fail-closed. If a test is red, fix the
  code or the fixture, never the check.

## Remaining work, in order

Do them one at a time; one commit per sub-unit; stop cleanly at a sub-unit
boundary if you run out of budget.

1. **Terminal provenance.** The terminal path takes provenance from the original
   approval plus the original ledger, not from the report. `ArtifactCommit` and
   the attempt manifest require complete source metadata; an exact replay must
   not change policy or expiry; new staging and body writes reject expired
   material; old records with missing fields are not upgraded to `Full`.
   Test fixtures grant provenance explicitly.
2. **L0 / final source catalog** and cleanup of the original staging area.
3. **Producer side in `studiolo-local-server`:** fixed configuration, the Unix
   socket listener, and scheduling of backup jobs.
4. **`studiolo-backup-worker` resident entry:** the crate is a library today
   (no `main.rs`). Add the long-running main loop and the deployment templates
   (systemd unit(s) under `deploy/`, following the style and contract tests of
   the existing `deploy/` subdirectories). Templates only — nothing is installed.
5. **Restore orchestration into a new root** (all owners, both ledgers).
6. **Reclaim / retention timer wiring.**
7. **End-to-end test and docs:** seal → back up to a local restic repository →
   restore into a new root → reopen every database. Update the docs under
   `docs/` that describe the backup chain.

For each sub-unit, write its completion criteria at the top of your commit
message body before you start coding it.

## Out of scope / forbidden

- Production: no deployment, no real NAS / restic remote, no real credentials,
  no dual-UID production drill, no RTO drill. These need the owner in person.
- Do not touch: `.github/workflows/**`, the repository variable
  `CI_RUNNER_LABELS`, `AGENTS.md`, `CLAUDE.md`, `crates/observability/**`,
  `crates/local-server/src/alert_main.rs`, any new `doorbell` files,
  `deploy/alerts/**`, `web/**`. Other sessions own them.
- Do not add `spawn_blocking` / `block_in_place` anywhere under
  `crates/agent-server/src` (a guard forbids it; see BUG-034 in the record).
- Do not weaken the architecture guards in
  `crates/studiolo/tests/architecture_*`. When you change a guarded file's
  filesystem calls, update the exact anchors and allowed-token counts in the
  matching guard in the same commit (the guard compares whitespace-stripped
  source; a trailing comma changes the token).
- Do not download or install binaries. If `restic` is missing on your VM, the
  tests that need the official CLI stay unverified — say so in the report.
- No secrets, tokens, internal host addresses or personal data in code, tests,
  docs or commit messages.

## How to verify (Cursor Cloud VM)

Read `AGENTS.md` section 六 first. Everything except the frontend goes through
`scripts/test-cursor-cloud.sh`:

```bash
scripts/test-cursor-cloud.sh cargo --locked fmt --all -- --check
scripts/test-cursor-cloud.sh cargo --locked clippy --workspace --all-targets -- -D warnings
scripts/test-cursor-cloud.sh cargo --locked test -p studiolo-backup -p studiolo-backup-worker -p studiolo-checkpoint-storage -p studiolo-control-db -p studiolo-task-engine --no-fail-fast
scripts/test-cursor-cloud.sh cargo --locked test -p studiolo --test architecture
scripts/test-cursor-cloud.sh cargo --locked test -p studiolo-local-server --no-fail-fast
```

Toolchain: Rust 1.97.1 (pinned by `rust-toolchain.toml`). The two vault-broker
tests named in `AGENTS.md` section 六 are known to fail on the VM; ignore them.
The main session re-runs everything on the project's own test machine before
merging, so report honestly what you ran and what you could not run.

## Conventions

- Code comments, test names where the file already uses Chinese, and commit
  messages are written in Chinese (see `git log`). Identifiers stay English.
- One function does one thing; side effects (files, network, time) are kept
  apart from pure logic; constants live in one place.
- Every fix is the complete fix: no temporary patches, no bypasses.

## Final report (required)

Reply with: branch name, head commit, one line per sub-unit
(`done` / `partial` / `not started`) with the commands you ran and their
result lines, every decision you made on your own, anything left open, and
anything that needs the owner in person.

## Status after the first hand-back (2026-11-15, main session)

The cloud agent pushed eight commits to `cursor/t204-backup-chain` (head `34f19ea`). It ran on `grok-4.7`, not the
requested model. The main session reviewed the full diff and verified on the x86_64 test node.

**Landed on `claude/project-code-completion-40e514`** (sub-units 1 and 2 only):

- `24af3b2` — terminal commit takes provenance from the original approval and ledger (cherry-picked from `fa31775`).
- `8d4138c` — final source catalog; original staging is cleaned after commit (cherry-picked from `def7125`).
- `000efae` — the export scanner was **rewritten** by the main session. The agent's scanner rejected every real
  export (it parsed `meta-v1/catalogs/feed-v1-N.json` as a history catalog) and its tests only used hand-built trees.
  The rewrite follows the real six-owner layout, adds `local-server::checkpoint::trusted_catalog`, and is exercised by
  the real seal test (mode 4 in `crates/local-server/tests/coordinator/checkpoint_owners/seal.rs`).

**Not landed** (sub-units 3–7 stay on `cursor/t204-backup-chain`; do not cherry-pick them as they are):

| Commits | Why it was not taken |
|---|---|
| `7477cb6` producer config / listener / schedule check | The listener is not wired into `serve()`; there is no scheduling loop. |
| `c8701f2` worker `main.rs`, `queue.rs`, service template | Bypasses the existing `BackupWorker` state machine (no per-file remote re-verification, no final-manifest commit, no audit); `harden()` runs per batch instead of at start; the queue lives in the worker's own 0700 state directory, so the producer cannot enqueue. |
| `7c1de3e`, `34f19ea` new-root restore | Uses `ControlDb::open`, which silently creates an empty database when the file is missing; covers 6 of the 11 owner directories; presence is reported by the caller instead of read from the sealed manifest. |
| `1145a21` reclaim timer | Runs reclaim inside the worker binary; `reclaim_staging` is producer-only by contract. |
| `7e67ecd` end-to-end test and docs | `local_roundtrip.rs` only asserts the restic version string. |

**Open decisions (owner: the user) before sub-units 3–7 can be redone:**

1. How a job crosses the two system accounts (a read-only "next pending key" command on the sealed protocol, or a
   producer-owned queue directory readable by the worker's group).
2. The new-root restore protocol for Auth, Bridge, Transport and Plane (Auth needs session config, clock and entropy;
   Bridge needs an explicit mode and a new-root binding restore).
3. How attempt directories with a version-1 manifest enter a backup (today the whole seal fails closed).
4. Whether the "freeze all owners" schedule lives in the main service process.

Known limits are recorded in `docs/m2/checkpoint-recovery-contract.md` ("已知限制，上线前要处理"). The restic
end-to-end drill has not been run (the test node has no pinned restic 0.19.1 and nothing is downloaded there).

## Status after the redo (2026-11-15 evening, main session)

The four open decisions above were settled by the main session on the user's behalf (the user asked for everything to
be finished in one session and can still overturn any of them): (1) a read-only "next pending key" command on the
sealed protocol; (2) new-root restore fails closed; (3) version-1 manifests stay rejected; (4) no timer inside the main
service — a one-shot command started by a system timer.

Sub-units 3–7 were then redone, one commit each, verified on the x86_64 test node:

| Sub-unit | Commit | Where to read |
|---|---|---|
| Producer wired into the main service | `916e01d` | `docs/contracts/backup-producer.md` |
| Worker long-running entry (`run` / `schedule` / `init`), deploy templates | `138d44c` | `docs/contracts/backup-worker-process.md` |
| New-root restore (retrieved copy, byte-checked placement, roster-driven orchestration) | `a85732b` | `docs/contracts/new-root-restore.md` |
| Exit for expired `committing` attempts | `56781fa`, `90443a7` | `docs/m2/checkpoint-recovery-contract.md` ("已知限制") |
| In-process end-to-end drill (source-expired path, no restic) | `2966f15` | `docs/contracts/backup-worker-process.md` ("端到端演练做到哪") |

Still **not done** (each needs its own task; do not start them from this file):

- New-root restore protocols for Control, AgentStore, Policy and Bridge. Today a sealed checkpoint that contains any of
  them is refused before anything is written, so a deployment with the board cannot be restored through this path.
  How post-cutoff revocations are reconciled for the policy store is a security decision that belongs to the user.
- A command-line entry for the restore orchestration, and encrypted storage for the recovery kit.
- Broker cleanup of atomic-write leftovers at open.
- Deleting already-published source bodies after their retention ends (the Broker has no delete capability).

Still **not verified**: the upload / re-verification / final-manifest leg and the retrieval leg against a real pinned
restic (none on the test node, nothing is downloaded there); a full round with two system accounts, systemd
credentials and the timer on a real machine; Auth offline restore and the final cut-over after a new-root restore;
the "resume an interrupted restore" path (no test).

## Review and the user's rulings (2026-11-16)

An independent review (seven agents) of the four decisions above was run, and the user ruled on the four trade-offs it
raised. The code was changed accordingly on the same branch; the contracts named above are authoritative:

1. A failed seal is retried only after the full approved interval (the 60-second retry floor is gone). The main service
   also reclaims expired staging on its own every 15 minutes (reclaim only, no freeze), so stopped timers no longer
   leave expired copies on disk; the busy flag is reset even if the background thread panics.
2. New-root restore is now refused as a whole whenever Auth is present (every production checkpoint), because there is
   no production-usable Auth revocation entry and no "cannot open before revocation" gate yet. Placement, rebind and
   the ledger-tail check stay, tested only through a test-only bypass. Restoring is therefore unavailable until a
   follow-up task adds the revocation entry and the gate.
3. Expired, never-started candidate commits are **no longer failed by reconciliation**. They stay pending with their
   staged bodies kept; only the owner entry `discard_expired_candidates` (task engine, not yet exposed over HTTP or in
   the cockpit) settles them as failed and discards the staging. Until the owner acts, control-db export and the whole
   seal stay blocked; the user accepted that cost.
4. Version-1 attempt directories and pre-upgrade staging stay rejected; before go-live, run a read-only check on the
   production vault from the Mac (see `deploy/backup-worker/README.md`).
