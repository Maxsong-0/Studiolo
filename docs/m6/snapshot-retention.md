# Code snapshot reuse and explicit orphan retention

Authority: Vault ADR-018 §1–2. This document describes the implementation contract, not a completed production cleanup or RG-RUNNER acceptance receipt.

## Shared baseline, separate writes

`CodeSnapshot::create(source, attempt)` is a trusted host API. The internal `Snapshot` alias keeps `RealRunner` unchanged. Both arguments must be canonical absolute directories. The caller supplies a new empty attempt directory beneath a private, current-UID-owned `0700` scratch root and holds its source/landing admission lease. A per-root OS file lock serializes snapshot registration and cleanup; a busy lock fails closed.

Each capture uses its own temporary Git index and recomputes the full tracked, dirty and nonignored untracked tree. The source checkout index and branch references are unchanged. Git already stores trees and blobs by content in its shared object database: attempts with identical bytes therefore share their immutable base tree. The new private metadata cache additionally reuses an anonymous commit for at most 60 seconds only when canonical source/repository, exact full tree and HEAD all match. Reuse validates the cached commit's actual tree and parent. Changed content or HEAD is a cache miss; malformed or missing cached objects reject reuse.

This does **not** skip content validation based on a timestamp, directory mtime, repository name or an old clean-HEAD cache. Each attempt still gets an independent detached writable work copy. Baseline computation still enumerates the current checkout; the optimization shares immutable Git objects and commit metadata, not mutable work directories.

`CodeSnapshot::invalidate_cache(attempt_root)` explicitly clears cache metadata after any trusted landing, including a no-op landing. It does not mutate already bound attempts or delete Git objects. The owner-only landing composition invalidates every configured provider cache before applying code.

Before excluding the in-sandbox `.git` control file, capture locks the exact Git worktree registration. This prevents Git from pruning live snapshot registrations simply because the control file is intentionally absent. The host writes a `0600` `.studiolo-snapshot.json` in the attempt parent, outside the mounted `/work`, with source/repository/base, creation time and root/attempt/work/Git-registration device/inode/owner identities. The record is fsynced before capture succeeds.

## Durable retention

`CodeSnapshot::preserve(retained)` retains the binary patch using a private temporary file, fsync, create-only publication and directory fsync. A companion `<patch-name>.json` durably stores base tree, after tree, SHA-256 and retained path. Repeating identical preservation is idempotent; changed patch or metadata never overwrites earlier evidence. Patch retention is bounded at 16 MiB. Candidate scans reject special files, symbolic links, hardlinks and control paths, with aggregate byte and node limits.

Conflict-marker patches are preserved but return `CandidateRejected`; they are never treated as accepted changes. The cleanup API consequently leaves those work copies for explicit recovery.

## Explicit cleanup API

`cleanup_orphan_snapshots(&SnapshotCleanupRequest)` receives:

- approved `source_checkout`, private canonical `attempt_root`, and a separate durable private `retained_root`;
- the exact, freshly read `active_attempts` directory paths;
- trusted host `now_unix_seconds` and `minimum_age_seconds` of at least 300.

The caller must stop new admissions and hold the maintenance/landing window, confirm old scopes are isolated, and obtain the active set from authoritative control state. Age is **not** a process-liveness test. No Runner-provided path or self-reported active set belongs in this API. It is not called implicitly from `Drop`, a read endpoint, a startup boolean or this subtask's tests against user directories.

The sweep is bounded to 4096 root entries. It protects all listed active paths and all young/future-dated registrations. Unknown directories, legacy copies without registration, corrupt records, replaced identities, mismatched source or Git registration, unsafe trees, insufficient retention access and preservation conflicts are reported as `blocked` and retained. It does not guess that an unknown directory is disposable.

For an old, inactive registered copy, cleanup first validates the complete tree and exact Git registration, then durably preserves `<attempt-name>-orphan-unlanded.patch` and its metadata outside scratch. It fsyncs the retention pointer in the attempt record, rechecks directory identities, and only then removes the work copy plus **that exact** Git worktree registration. It never runs repository-wide `git worktree prune` or deletes retained evidence. `removed`, `retained`, `protected` and `blocked` are returned separately.

A filesystem failure during deletion can leave a partially cleaned directory or Git registration. Retained patch and metadata survive; the next sweep fails closed if it cannot re-establish the original identities. This is preservation-before-cleanup, not an atomic filesystem-delete transaction. Operators must resolve such blocked remnants explicitly.

## Integration and validation boundary

Public exports: `CodeSnapshot`, `SnapshotCleanupRequest`, `SnapshotCleanupReport`, `cleanup_orphan_snapshots`. Existing `RealRunner` snapshot creation and preservation use the new behavior automatically. RunnerPlane performs an audited startup sweep before socket binding and admissions. Every root entry starts protected; only a durably terminal attempt with an exact trusted RunnerRef and confirmed scope isolation is eligible. Source repository leases cover cleanup. Unknown references, active/T1 attempts and unregistered directories remain protected. No production directories have been cleaned by this change.

`m6_runner_contract.rs` adds disposable Git-repository contracts for full dirty/untracked baseline reuse, independent writes and unchanged source index/HEAD; invalid cached commit rejection; active/young protection and exact old-orphan retention; unknown/replaced/symlinked directory refusal; symlink/hardlink and conflicting-retention refusal; conflict patch retention; and invalid cleanup-root/age inputs. Root integration runs the runner contract tests and strict Clippy on Linux. These tests do not require provider credentials, a real CLI, network access or production source files.
