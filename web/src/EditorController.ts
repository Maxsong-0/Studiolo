import { ApiError } from './sessionHttp';
import {
  readAfterCommit,
  readCanonical,
  replaceCanonical,
} from './canonicalApi';
import {
  byteLength,
  canEdit,
  EDIT_LIMIT,
  revisionConflict,
} from './canonicalModel';
import type { PendingMutation } from './canonicalModel';
import type { HistoryFile } from './historyModel';
import { EditorStateCore, editorMessage as message } from './EditorStateCore';
export type { EditorSnapshot } from './EditorStateCore';
const code = (error: ApiError) =>
  error.details && typeof error.details === 'object' && 'code' in error.details
    ? error.details.code
    : null;

/** 单文档请求拥有者：一次保存绑定一个不可变正文与幂等键，响应不覆盖后续输入。 */
export class EditorController extends EditorStateCore {
  /** 离开确认框开着时为 true：人正在决定草稿去留，停顿计时不发起自动保存。 */
  private autosaveHeld = false;
  /**
   * 暂停或恢复停顿计时的自动保存。只管这一个计时：不改保存阶段，不取消在途请求，
   * 显式保存照常；失焦保存由界面的不保存区（data-editor-no-save）把关。恢复时按当前阶段重新排定。
   */
  holdAutosave = (held: boolean) => {
    this.autosaveHeld = held;
    this.schedule();
  };
  protected override schedule() {
    if (this.autosaveHeld) clearTimeout(this.timer);
    else super.schedule();
  }
  async save(recover = false) {
    clearTimeout(this.timer);
    const { base, draft, phase, pending } = this.state;
    if (
      !base ||
      !this.writesAllowed() ||
      this.stopped ||
      this.composing ||
      this.reading ||
      phase === 'saving' ||
      phase === 'verifying'
    )
      return;
    if (
      recover
        ? phase !== 'unknown' || !pending
        : phase !== 'dirty' || !canEdit(base)
    )
      return;
    const request = recover
      ? pending!
      : {
          kind: 'replace' as const,
          vault_uuid: base.vault_uuid,
          path: base.path,
          expected_revision: base.revision,
          content: draft,
          idempotency_key: crypto.randomUUID(),
        };
    await this.commit(request, recover);
  }
  canRestoreHistory = () => {
    const { base, draft, phase, pending } = this.state;
    return Boolean(
      base &&
      this.writesAllowed() &&
      this.store.ownerScope &&
      !this.store.get(base) &&
      phase === 'clean' &&
      !pending &&
      !this.state.scopeChanged &&
      draft === base.content &&
      canEdit(base) &&
      !this.stopped &&
      !this.composing &&
      !this.reading,
    );
  };
  beginFileOperation = (revision: string) => {
    if (!this.canRestoreHistory() || this.state.base?.revision !== revision)
      return null;
    clearTimeout(this.timer);
    const base = this.state.base;
    this.set({
      phase: 'file-operation',
      message: '本页已暂停编辑。文件操作回执与原请求在下方保留。',
    });
    return base;
  };
  restoreHistory = async (version: HistoryFile) => {
    const base = this.state.base;
    if (
      !base ||
      !this.canRestoreHistory() ||
      version.vault_uuid !== base.vault_uuid ||
      version.path !== base.path ||
      version.revision === base.revision ||
      version.revision.length !== 64 ||
      !/^[0-9a-f]{64}$/.test(version.revision) ||
      byteLength(version.content) > EDIT_LIMIT ||
      /[\0\uD800-\uDFFF]/u.test(version.content)
    )
      return;
    await this.commit(
      {
        kind: 'restore',
        vault_uuid: base.vault_uuid,
        path: base.path,
        expected_revision: base.revision,
        source_revision: version.revision,
        idempotency_key: crypto.randomUUID(),
        content: version.content,
      },
      false,
    );
  };
  private async commit(request: PendingMutation, recover: boolean) {
    const { base, draft } = this.state;
    if (!base) return;
    if (!this.store.canKeep({ base, draft, pending: request })) {
      this.limit();
      return;
    }
    this.set({
      pending: request,
      phase: 'saving',
      message:
        request.kind === 'restore'
          ? '正在恢复历史版本并等待审计回执…'
          : '正在保存本次提交…',
      remote: null,
    });
    try {
      const receipt = await replaceCanonical(request, this.controller.signal);
      if (this.stopped) return;
      this.set({
        base: {
          ...base,
          content: request.content,
          revision: receipt.new_revision,
        },
        pending: null,
        ...(request.kind === 'restore' && this.state.draft === base.content
          ? { draft: request.content }
          : {}),
        phase: 'verifying',
        message: '已取得持久保存回执；正在另行核对当前磁盘版本。',
      });
      await this.verify();
    } catch (error) {
      if (this.stopped) return;
      if (recover) {
        this.set({
          phase: 'unknown',
          message: `原请求仍未取得持久回执，继续保留同一标识与正文。${message(error)}`,
        });
        return;
      }
      const conflict =
        error instanceof ApiError && error.status === 409
          ? revisionConflict(error.details, request)
          : null;
      if (conflict) {
        this.set({
          pending: null,
          phase: 'conflict',
          message: '保存遇到版本冲突，基线与草稿均已保留。正在读取当前版本。',
        });
        await this.inspect();
      } else if (
        error instanceof ApiError &&
        error.status < 500 &&
        error.status !== 409
      ) {
        this.set({ pending: null, phase: 'blocked', message: message(error) });
      } else if (
        error instanceof ApiError &&
        code(error) === 'audit_unavailable'
      ) {
        this.set({
          pending: null,
          phase: 'blocked',
          message:
            '恢复审计不可用，服务已确认本次恢复未安装。请维护恢复后重新核验；原编辑内容已保留。',
        });
      } else if (error instanceof ApiError && code(error) === 'stale') {
        this.set({
          pending: null,
          phase: 'blocked',
          message: '快照尚未追平。请重新核验后再明确继续保存。',
        });
      } else {
        this.set({
          phase: 'unknown',
          message:
            '保存结果不确定。请保留草稿；只能核对现版本或恢复原请求，不能换键重试。',
        });
      }
    }
  }
  private async verify() {
    try {
      const remote = await readAfterCommit(this.path, this.controller.signal);
      if (this.stopped) return;
      const base = this.state.base;
      if (!base || remote.vault_uuid !== base.vault_uuid) {
        this.set({ scopeChanged: true });
        throw new Error('Vault 身份发生变化，请保留草稿并维护核对。');
      }
      if (remote.revision !== base.revision) {
        this.set({
          remote,
          phase: 'conflict',
          message:
            '保存回执已确认，但另行读取的磁盘版本已有变化，请比较后继续。',
        });
      } else {
        if (remote.content !== base.content)
          throw new Error('相同版本的正文证据不一致，不能确认当前内容。');
        if (remote.freshness?.stale || remote.editing.reason === 'stale') {
          this.set({
            base: remote,
            remote: null,
            phase: 'blocked',
            message:
              '本次保存已提交；完整视图尚未追平，请稍后手动核对当前磁盘版本。',
          });
          return;
        }
        this.set({
          base: remote,
          remote: null,
          phase:
            this.state.draft === remote.content
              ? 'clean'
              : canEdit(remote)
                ? 'dirty'
                : 'blocked',
          message: '本次保存已提交，并已另行读取当前磁盘版本。',
        });
        this.schedule();
      }
    } catch (error) {
      if (!this.stopped)
        this.set({
          phase: 'blocked',
          message: `保存回执已确认；当前磁盘尚未核对。${message(error)}`,
        });
    }
  }
  inspect = async () => {
    if (this.reading || this.stopped || this.state.phase === 'saving') return;
    this.reading = true;
    try {
      const remote = await readCanonical(this.path, this.controller.signal);
      if (this.stopped) return;
      if (remote.vault_uuid !== this.state.base?.vault_uuid) {
        this.set({ scopeChanged: true });
        throw new Error('Vault 身份发生变化，不能合并此草稿。');
      }
      this.set({
        remote,
        message: '当前版本已另行读取；请比较后明确选择下一步。',
      });
    } catch (error) {
      if (!this.stopped)
        this.set({
          remote: null,
          message: `当前版本读取失败，草稿仍保留。${message(error)}`,
        });
    } finally {
      this.reading = false;
    }
  };
  continueFromCurrent = () => {
    const remote = this.state.remote;
    if (!remote || this.state.pending || !canEdit(remote)) return;
    this.set({
      base: remote,
      remote: null,
      phase: this.state.draft === remote.content ? 'clean' : 'dirty',
      message: '已按当前版本重新建立保存基线。',
    });
    this.schedule();
  };
}
