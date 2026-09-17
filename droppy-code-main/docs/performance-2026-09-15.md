# Droppy Code performance pass, 15 September 2026

Three passes over the whole app: process I/O, provider transport, the runtime, storage,
git, rendering and search. The work is split into small merge requests, each with the tests
that pin it. `scripts/run_tests.sh` builds the dev app and runs all seven suites
(117 tests); no suite launches the app or calls a paid provider.

## What changed

| Area | Change | Where |
|---|---|---|
| Process I/O | A cancelled shell command stops its child process; timeouts escalate to the process group; a finished command releases its pipes and its deadline task at once. | `Support/Shell.swift` |
| Transport | Stdio frames are parsed incrementally (line and Content-Length), so a fragmented message is scanned once, not once per chunk. Cancelled JSON-RPC calls give their continuation back. | `Support/StdioFramer.swift`, `JSONRPC.swift` |
| Git | The index path of a worktree is cached while its `.git` file stands; a snapshot no longer spawns `git rev-parse` per ask. Checkpoint refs are deleted in one `update-ref --stdin` batch. The working-tree watch waits on the FSEvents cookie, not a 1 ms poll. | `Git/Git.swift`, `Git/WorkingTreeWatch.swift` |
| Turn diffs | A turn's files come from one change list: the checkpoint diff, plus what its tools reported outside the checkout. The revert preview and the file cards read the same summary. | `Git/TurnDiff.swift`, `Git/RevertPreview.swift`, `Models/FileChangeSummary.swift` |
| Runtime | A turn is finalised once, by the first of its completion and exit; a queued completion cannot finish a replacement turn; only the session observed last is listened to. Stopping a session ends its turn as interrupted. Attachments are read and base64-encoded off the main actor. | `Runtime/ThreadRuntime.swift` |
| Storage | One serial writer owns saves, the final save and deletion, so a queued write cannot undo a delete. Prefetch is bounded by count and by bytes. A thread already on disk as it is in memory is not written again at quit. | `Store/Storage.swift` |
| DeepSeek, Meta | The tool loop is one cancellable task: a stop reaches a running command and a waiting approval. File tools are shared and read at most 1,000,001 bytes before refusing a large file. | `Providers/NativeFileTools.swift` |
| Sidebar | Thread positions, projects and children are indexed once per library change instead of scanned per row. | `App/AppModel.swift` |
| Composer | File search scores on the cooperative pool, ASCII paths on their bytes, and keeps only the top eight. Slash commands and Codex skills are discovered per project directory before the first prompt. | `Views/Composer/ComposerView.swift` |
| Markdown | Messages stream through incremental parsing; a bounded window of history is parsed ahead of the scroll. | `Views/Markdown/MarkdownView.swift`, `MarkdownParser.swift` |
| Diff inspector | The collapsed view stops at its 200-line budget before grouping. | `Views/Diff/DiffInspector.swift` |
| Energy | The effort slider's track effect and the running turn's elapsed-time clock pause while nothing can see them. The update progress creep ends at its ceiling instead of ticking forever. Thumbnails live in size-bounded caches. | `Views/Composer/EffortSlider.swift`, `Views/Chat/ThreadTimeline.swift`, `App/AppUpdater.swift`, `Support/ThumbnailCache.swift` |
| Text | Em-dash cleanup rejects a string by its UTF-8 bytes before walking graphemes; the first line of a command's output is found without splitting the rest. | `Support/Text.swift` |

## Measured

Apple M2 Pro, 16 GiB, macOS 27.0, Xcode 27.0. Synthetic workloads, medians of 30 runs after
warm-up; other apps were running. These are not app-wide numbers.

- Open descriptors after 100 short commands: 203 before, 3 after (`scripts/benchmark_performance.sh`).
- One 256 KiB JSON message delivered in 128-byte chunks: about 1 s before, 3.5 ms after (same script, run on 15 September 2026).
- Rejecting a 128 MiB sparse file: 21.8 ms and 144 MB peak RSS before, 1.1 ms and 13 MB after (`Tests/RuntimePerformanceTests.swift`, `boundedReadRejectsOversizedSparseFiles`).
- Fuzzy file match, 20,000 paths × 10: 0.60 s on graphemes, 0.14 s on ASCII bytes, same answers on 28,985 fixtures (`scripts/benchmark_file_matcher.sh`, run on 15 September 2026).
- Unchanged-worktree snapshot, 4,000 files: 23.5 ms before, 11.3 ms after with the cached index path.
- Draft emptiness on an 8 KB draft × 1,000: 0.39 ms before, 0.05 ms after.

Unmeasured, by reasoning only: SSE parsing off the main actor, attachment encoding, the
slider and clock pausing, the Genie sample bound, the bounded Markdown warm-up. Rebuilding
em-dash-free strings by per-character append measured slower and was reverted.

## Not done

- No Instruments capture of launch, scrolling, GPU or wakeups; the source-level work above is not evidence of a measured battery gain.
- Liquid Glass surfaces (the window backdrop, the veil, the panels) still resample every frame a reply streams. See E1 in [the audit](audit-2026-09-15.md).
- Retained runtimes keep their history; the idle timer stops sessions only.
