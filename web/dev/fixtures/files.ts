/**
 * 可写的假 Vault：权威原文（`/api/v1/file`）、保存与历史恢复（`/api/v1/mutations`）、页面历史
 * （`/api/v1/history`、`/history/file`）、文件操作（删除、重命名、从最近删除恢复）与墓碑分页。
 *
 * revision 按「Vault UUID ＋ 路径 ＋ 原文」算 SHA-256，同一份原文放回原路径得到同一个 revision，
 * 与恢复回执的核对口径一致。形状以 `canonicalModel.ts`、`historyModel.ts`、
 * `fileOperationValidation.ts`、`renameValidation.ts` 为准。
 */
import { DAY, HOUR } from './clock.ts';
import { VAULT_UUID, sha256 } from './ids.ts';
import { DELETED_NOTE } from './vault.ts';
import type { VaultNote } from './vault.ts';

const OWNER = {
  kind: 'human',
  issuer: 'https://tengqing.cloudflareaccess.com',
  subject: '主人',
} as const;
const OBSIDIAN = { kind: 'obsidian' } as const;
type Actor = typeof OWNER | typeof OBSIDIAN;

export const EDIT_LIMIT = 262_144;
const bytesOf = (text: string) => new TextEncoder().encode(text).length;
export const revisionOf = (path: string, content: string) =>
  sha256(`${VAULT_UUID}\n${path}\n${content}`);

interface HistoryEntry {
  id: string;
  revision: string;
  content: string;
  authored_at_ms: number | null;
  author: Actor | null;
  archived_at_ms: number | null;
  archived_by: Actor | null;
}

interface Tombstone {
  id: string;
  path: string;
  content: string;
  deleted_at_ms: number;
  deleted_by: Actor;
}

export type Outcome = { status: number; body?: unknown };
const problem = (status: number, code: string, error: string): Outcome => ({
  status,
  body: { code, error },
});

/** 去掉最后一个非空行，模拟更早的一版。 */
function earlier(content: string) {
  const lines = content.replace(/\n+$/, '').split('\n');
  lines.pop();
  return `${lines.join('\n')}\n`;
}

const presence = (path: string, content: string) => ({
  kind: 'present',
  revision: revisionOf(path, content),
  sha256: sha256(content),
  bytes: bytesOf(content),
});

export class VaultStore {
  private history = new Map<string, HistoryEntry[]>();
  private tombstones: Tombstone[];
  private operations = new Map<
    string,
    { fingerprint: string; outcome: Outcome }
  >();
  private sequence = 1_207;

  constructor(
    readonly notes: VaultNote[],
    start: number,
    private readonly changed: () => void,
  ) {
    for (const note of notes) {
      const previous = earlier(note.content);
      const oldest = earlier(previous);
      this.history.set(note.path, [
        this.entry(
          note.path,
          previous,
          start - 2 * DAY,
          OWNER,
          start - 6 * HOUR,
          OWNER,
        ),
        this.entry(
          note.path,
          oldest,
          start - 5 * DAY,
          OBSIDIAN,
          start - 2 * DAY,
          OWNER,
        ),
      ]);
    }
    this.tombstones = [
      {
        id: sha256(`tombstone ${DELETED_NOTE.path}`),
        path: DELETED_NOTE.path,
        content: DELETED_NOTE.content,
        deleted_at_ms: start - 2 * DAY,
        deleted_by: OBSIDIAN,
      },
    ];
  }

  private entry(
    path: string,
    content: string,
    authored: number | null,
    author: Actor | null,
    archived: number,
    archivedBy: Actor,
  ): HistoryEntry {
    const revision = revisionOf(path, content);
    return {
      id: `hist:${revision.slice(0, 24)}:${archived}`,
      revision,
      content,
      authored_at_ms: authored,
      author,
      archived_at_ms: archived,
      archived_by: archivedBy,
    };
  }

  note(path: string) {
    return this.notes.find((note) => note.path === path);
  }

  canonical(path: string, freshness: { stale: boolean }) {
    const note = this.note(path);
    if (!note) return null;
    const allowed = !freshness.stale && bytesOf(note.content) <= EDIT_LIMIT;
    return {
      vault_uuid: VAULT_UUID,
      path,
      content: note.content,
      revision: revisionOf(path, note.content),
      editing: {
        allowed,
        reason: allowed ? null : freshness.stale ? 'stale' : 'too_large',
        max_bytes: EDIT_LIMIT,
      },
      freshness,
    };
  }

  /** 同一个幂等键：原样重放；键相同内容不同：冲突。 */
  private once(key: string, request: unknown, run: () => Outcome): Outcome {
    const fingerprint = JSON.stringify(request);
    const prior = this.operations.get(key);
    if (prior)
      return prior.fingerprint === fingerprint
        ? prior.outcome
        : problem(
            409,
            'idempotency_conflict',
            '此请求标识已用于不同内容，请保留草稿并核对。',
          );
    const outcome = run();
    if (outcome.status === 200)
      this.operations.set(key, { fingerprint, outcome });
    return outcome;
  }

  mutate(
    request: Record<string, unknown>,
    now: number,
    stale: boolean,
  ): Outcome {
    const path = String(request.path ?? '');
    const kind = request.kind;
    if (
      (kind !== 'replace' && kind !== 'restore') ||
      typeof request.idempotency_key !== 'string' ||
      typeof request.expected_revision !== 'string'
    )
      return problem(400, 'invalid', '请求格式无效。');
    if (request.vault_uuid !== VAULT_UUID)
      return problem(403, 'forbidden', '目标 Vault 与当前会话不一致。');
    return this.once(request.idempotency_key, request, () => {
      if (stale)
        return problem(409, 'stale', '内容尚未追平，请重新核验后再保存。');
      const note = this.note(path);
      if (!note) return problem(404, 'not_found', '文档不存在。');
      const actual = revisionOf(path, note.content);
      if (actual !== request.expected_revision)
        return {
          status: 409,
          body: {
            code: 'revision_conflict',
            error: '磁盘版本已变化，请比较双方内容。',
            vault_uuid: VAULT_UUID,
            path,
            expected_revision: request.expected_revision,
            actual_revision: actual,
          },
        };
      let content: string;
      if (kind === 'replace') {
        if (
          typeof request.content !== 'string' ||
          bytesOf(request.content) > EDIT_LIMIT
        )
          return problem(413, 'too_large', '文档超出网页编辑限制。');
        content = request.content;
      } else {
        const source = this.history
          .get(path)
          ?.find((entry) => entry.revision === request.source_revision);
        if (!source) return problem(404, 'not_found', '历史版本不存在。');
        content = source.content;
      }
      this.history
        .get(path)!
        .unshift(this.entry(path, note.content, null, OWNER, now, OWNER));
      note.content = content;
      this.changed();
      return {
        status: 200,
        body: {
          status: 'committed',
          receipt: {
            path,
            previous_revision: actual,
            new_revision: revisionOf(path, content),
          },
        },
      };
    });
  }

  historyPage(vault: string | null, path: string) {
    const entries = vault === VAULT_UUID ? this.history.get(path) : undefined;
    if (!entries || !this.note(path)) return null;
    return {
      vault_uuid: VAULT_UUID,
      path,
      entries: entries.slice(0, 20).map((entry) => ({
        id: entry.id,
        revision: entry.revision,
        bytes: bytesOf(entry.content),
        authored_at_ms: entry.authored_at_ms,
        author: entry.author,
        archived_at_ms: entry.archived_at_ms,
        archived_by: entry.archived_by,
      })),
      next_cursor: null,
    };
  }

  historyFile(vault: string | null, path: string, revision: string | null) {
    const entry =
      vault === VAULT_UUID
        ? this.history.get(path)?.find((item) => item.revision === revision)
        : undefined;
    return entry
      ? {
          vault_uuid: VAULT_UUID,
          path,
          revision: entry.revision,
          content: entry.content,
        }
      : null;
  }

  tombstonePage() {
    return {
      entries: this.tombstones.map((item) => ({
        id: item.id,
        path: item.path,
        revision: revisionOf(item.path, item.content),
        sha256: sha256(item.content),
        bytes: bytesOf(item.content),
        deleted_at_ms: item.deleted_at_ms,
        deleted_by: item.deleted_by,
        retain_until_ms: item.deleted_at_ms + 30 * DAY,
      })),
      next_cursor: null,
    };
  }

  private receipt(key: string, effects: unknown[], tombstone: string | null) {
    this.sequence += 1;
    return {
      status: 200,
      body: {
        status: 'committed',
        receipt: {
          operation_id: tombstone ?? sha256(`operation ${key}`),
          sequence: String(this.sequence),
          outcome: 'committed',
          effects,
          tombstone_id: tombstone,
          evidence_id: null,
        },
      },
    };
  }

  fileOperation(request: Record<string, unknown>, now: number): Outcome {
    if (request.vault_uuid !== VAULT_UUID)
      return problem(403, 'forbidden', '目标 Vault 与当前会话不一致。');
    if (typeof request.idempotency_key !== 'string')
      return problem(400, 'invalid', '请求格式无效。');
    const key = request.idempotency_key;
    return this.once(key, request, () => {
      if (request.kind === 'delete') {
        const path = String(request.path);
        const note = this.note(path);
        if (!note) return problem(404, 'not_found', '文件不存在。');
        if (revisionOf(path, note.content) !== request.expected_revision)
          return problem(
            409,
            'revision_conflict',
            '磁盘版本已变化，请重新核对。',
          );
        const id = sha256(`operation ${key}`);
        this.remove(path);
        this.tombstones.unshift({
          id,
          path,
          content: note.content,
          deleted_at_ms: now,
          deleted_by: OWNER,
        });
        this.changed();
        return this.receipt(
          key,
          [
            {
              kind: 'confirmed',
              path,
              before: presence(path, note.content),
              after: { kind: 'missing' },
            },
          ],
          id,
        );
      }
      if (request.kind === 'restore_tombstone') {
        const path = String(request.path);
        const tombstone = this.tombstones.find(
          (item) => item.id === request.tombstone_id && item.path === path,
        );
        if (!tombstone) return problem(404, 'not_found', '删除记录不存在。');
        if (this.note(path))
          return problem(409, 'path_collision', '原路径已有文件，不能覆盖。');
        this.tombstones = this.tombstones.filter((item) => item !== tombstone);
        this.notes.push({ path, content: tombstone.content });
        this.history.set(path, []);
        this.changed();
        return this.receipt(
          key,
          [
            {
              kind: 'confirmed',
              path,
              before: { kind: 'missing' },
              after: presence(path, tombstone.content),
            },
          ],
          null,
        );
      }
      if (request.kind === 'rename') {
        const from = String(request.from);
        const to = String(request.to);
        const note = this.note(from);
        if (!note) return problem(404, 'not_found', '源文件不存在。');
        if (revisionOf(from, note.content) !== request.expected_revision)
          return problem(
            409,
            'revision_conflict',
            '磁盘版本已变化，请重新核对。',
          );
        if (this.note(to))
          return problem(409, 'path_collision', '目标路径已有文件。');
        const parent = to.split('/').slice(0, -1).join('/');
        if (
          parent &&
          !this.notes.some((item) => item.path.startsWith(`${parent}/`))
        )
          return problem(409, 'parent_missing', '目标父目录不存在。');
        const id = sha256(`operation ${key}`);
        const before = presence(from, note.content);
        this.tombstones.unshift({
          id,
          path: from,
          content: note.content,
          deleted_at_ms: now,
          deleted_by: OWNER,
        });
        note.path = to;
        this.history.set(to, this.history.get(from) ?? []);
        this.history.delete(from);
        this.changed();
        return this.receipt(
          key,
          [
            {
              kind: 'confirmed',
              path: to,
              before: { kind: 'missing' },
              after: presence(to, note.content),
            },
            {
              kind: 'confirmed',
              path: from,
              before,
              after: { kind: 'missing' },
            },
          ],
          id,
        );
      }
      return problem(400, 'invalid', '请求格式无效。');
    });
  }

  private remove(path: string) {
    const index = this.notes.findIndex((note) => note.path === path);
    if (index >= 0) this.notes.splice(index, 1);
  }
}
