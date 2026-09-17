import { readCanonical } from './canonicalApi';
import { byteLength, EDIT_LIMIT } from './canonicalModel';
import type { CanonicalDocument, PendingMutation } from './canonicalModel';
import type { DraftRecord, DraftStore } from './draftStore';

type Phase =
  | 'loading'
  | 'clean'
  | 'dirty'
  | 'saving'
  | 'verifying'
  | 'conflict'
  | 'unknown'
  | 'blocked'
  | 'restore'
  | 'file-operation'
  | 'review';
export interface EditorSnapshot {
  base: CanonicalDocument | null;
  draft: string;
  phase: Phase;
  message: string;
  remote: CanonicalDocument | null;
  pending: PendingMutation | null;
  scopeChanged?: boolean;
}
export const editorMessage = (error: unknown) =>
  error instanceof Error ? error.message : '操作未完成，请保留草稿。';

/** 草稿与计时生命周期；网络保存由同一个子类拥有，外部不能替换基线。 */
export abstract class EditorStateCore {
  protected state: EditorSnapshot = {
    base: null,
    draft: '',
    phase: 'loading',
    message: '',
    remote: null,
    pending: null,
  };
  private listeners = new Set<() => void>();
  protected controller = new AbortController();
  protected timer: ReturnType<typeof setTimeout> | undefined;
  protected composing = false;
  protected stopped = false;
  protected reading = false;
  constructor(
    readonly path: string,
    protected store: DraftStore,
    protected writesAllowed: () => boolean = () => true,
    private expectedVault?: string,
  ) {}
  abstract save(recover?: boolean): Promise<void>;
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  protected set(change: Partial<EditorSnapshot>) {
    this.state = { ...this.state, ...change };
    this.persist();
    this.listeners.forEach((listener) => listener());
  }
  private record(draft = this.state.draft): DraftRecord | null {
    return this.state.base
      ? { base: this.state.base, draft, pending: this.state.pending }
      : null;
  }
  private persist() {
    if (this.state.phase === 'restore') return;
    const record = this.record();
    if (!record) return;
    if (record.pending || record.draft !== record.base.content)
      this.store.keep(record);
    else this.store.remove(record.base);
  }
  async load() {
    this.stopped = false;
    this.controller.abort();
    this.controller = new AbortController();
    const signal = this.controller.signal;
    try {
      const base = await readCanonical(this.path, signal);
      if (this.stopped || signal.aborted) return;
      if (this.expectedVault && base.vault_uuid !== this.expectedVault) {
        this.set({ scopeChanged: true });
        throw new Error('目标文档属于不同 Vault，不能沿用重命名回执打开。');
      }
      this.set({
        base,
        draft: base.content,
        phase: this.store.get(base) ? 'restore' : 'clean',
        scopeChanged: false,
      });
    } catch (error) {
      if (!this.stopped && !signal.aborted)
        this.set({ phase: 'blocked', message: editorMessage(error) });
    }
  }
  canChange = (draft: string) => {
    const record = this.record(draft);
    return (
      this.state.phase !== 'file-operation' &&
      this.writesAllowed() &&
      byteLength(draft) <= EDIT_LIMIT &&
      !/[\0\uD800-\uDFFF]/u.test(draft) &&
      Boolean(record && this.store.canKeep(record))
    );
  };
  limit = () =>
    this.set({
      message:
        '输入未应用：文档上限为 256 KiB，草稿仓最多 8 篇／4 MiB。请先导出或处理已有草稿。',
    });
  change = (draft: string) => {
    if (this.stopped) return;
    if (!this.canChange(draft)) {
      this.limit();
      return;
    }
    const phase = ['clean', 'dirty'].includes(this.state.phase)
      ? draft === this.state.base?.content
        ? 'clean'
        : 'dirty'
      : this.state.phase;
    this.set({ draft, phase });
    this.schedule();
  };
  composition = (active: boolean) => {
    if (this.composing !== active) {
      this.composing = active;
      this.set({}); // IME 状态本身也使尚未提交的文件操作确认失效。
    }
    this.schedule();
  };
  blur = () => {
    if (!this.composing) void this.save();
  };
  protected schedule() {
    clearTimeout(this.timer);
    if (!this.stopped && !this.composing && this.state.phase === 'dirty')
      this.timer = setTimeout(() => void this.save(), 1500);
  }
  restore = () => {
    const current = this.state.base;
    const record = current && this.store.get(current);
    if (!record || !current) return;
    this.set({
      ...record,
      remote: current,
      phase: record.pending ? 'unknown' : 'review',
      message: '内存草稿已恢复。请比较当前磁盘版本后明确继续；不会自动提交。',
    });
  };
  discard = () => {
    if (this.state.phase === 'saving' || this.state.pending) return;
    const current = this.state.remote ?? this.state.base;
    if (!current) return;
    this.store.remove(current);
    this.set({
      base: current,
      draft: current.content,
      remote: null,
      phase: 'clean',
      message: '已放弃未保存草稿。',
    });
  };
  stop() {
    this.stopped = true;
    clearTimeout(this.timer);
    this.controller.abort();
    this.persist();
  }
}
