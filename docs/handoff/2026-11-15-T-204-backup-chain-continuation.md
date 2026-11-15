# T-204 backup chain — continuation handoff (2026-11-15)

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
