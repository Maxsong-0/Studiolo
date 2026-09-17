import { mutateFile } from './fileOperationApi';
import { ApiError } from './sessionHttp';
import type {
  FileOperationRecord,
  FileOperationRequest,
} from './fileOperationModel';

interface Shared {
  active: boolean;
  records: Map<string, FileOperationRecord>;
  running: Set<string>;
  listeners: Set<() => void>;
}
const code = (error: ApiError) =>
  error.details && typeof error.details === 'object' && 'code' in error.details
    ? error.details.code
    : null;
const declined = (error: unknown) =>
  error instanceof ApiError &&
  ((error.status < 500 && error.status !== 409) ||
    [
      'file_operation_conflict',
      'file_operation_cancelled',
      'idempotency_conflict',
      'audit_unavailable',
      'budget_exceeded',
      'stale',
    ].includes(String(code(error))));
const terminalRefusal = (error: unknown) =>
  error instanceof ApiError &&
  error.status === 409 &&
  ['file_operation_conflict', 'file_operation_cancelled'].includes(
    String(code(error)),
  );
/** 单页面内存保留原键，按主人隔离；关闭弹窗/失去会话不丢弃未决请求。 */
export class FileOperationStore {
  constructor(
    private shared: Shared = {
      active: true,
      records: new Map(),
      running: new Set(),
      listeners: new Set(),
    },
    readonly ownerScope: string | null = null,
  ) {}
  forScope(scope: string | null) {
    return new FileOperationStore(this.shared, scope);
  }
  activate() {
    this.shared.active = true;
  }
  destroy() {
    this.shared.active = false;
    this.shared.records.clear();
    this.notify();
  }
  subscribe = (listener: () => void) => {
    this.shared.listeners.add(listener);
    return () => {
      this.shared.listeners.delete(listener);
    };
  };
  private notify() {
    this.shared.listeners.forEach((listener) => listener());
  }
  getSnapshot = () =>
    this.ownerScope ? (this.shared.records.get(this.ownerScope) ?? null) : null;
  canStart = () =>
    Boolean(
      this.ownerScope &&
      this.shared.active &&
      !this.getSnapshot() &&
      !this.hasPending() &&
      this.shared.records.size < 8,
    );
  hasPending = () =>
    [...this.shared.records.values()].some((r) =>
      ['sending', 'uncertain'].includes(r.phase),
    );
  dismiss = () => {
    const value = this.getSnapshot();
    if (
      !this.ownerScope ||
      !value ||
      ['sending', 'uncertain'].includes(value.phase)
    )
      return;
    this.shared.records.delete(this.ownerScope);
    this.notify();
  };
  async submit(
    request: FileOperationRequest,
    signal: AbortSignal,
    restoreSource?: FileOperationRecord['restoreSource'],
  ) {
    const key = this.ownerScope;
    if (
      !key ||
      !this.shared.active ||
      signal.aborted ||
      this.shared.running.has(key)
    )
      return;
    const previous = this.getSnapshot();
    if (
      previous &&
      (previous.phase !== 'uncertain' ||
        JSON.stringify(previous.request) !== JSON.stringify(request))
    )
      return;
    if (!previous && !this.canStart()) return;
    const source = previous?.restoreSource ?? restoreSource;
    const immutable = Object.freeze({ ...request });
    const record: FileOperationRecord = {
      request: immutable,
      restoreSource: source && Object.freeze({ ...source }),
      phase: 'sending',
      message: previous
        ? '正在核对原请求回执…'
        : '正在等待文件操作的执行与审计回执…',
    };
    this.shared.records.set(key, record);
    this.shared.running.add(key);
    this.notify();
    try {
      const receipt = await mutateFile(immutable, signal, source);
      if (this.shared.active)
        this.shared.records.set(key, {
          ...record,
          receipt,
          phase: 'committed',
          message:
            request.kind === 'delete'
              ? '删除已提交，可在最近删除中恢复原路径。'
              : request.kind === 'rename'
                ? '重命名已提交；回执只记录原请求的结果，当前文件仍需独立读取。'
                : '恢复已提交；当前文件是否又有变化需另行读取。',
        });
    } catch (error) {
      const known = terminalRefusal(error) || (!previous && declined(error));
      if (this.shared.active)
        this.shared.records.set(key, {
          ...record,
          phase: known ? 'rejected' : 'uncertain',
          message: known
            ? `本次操作未执行。${error instanceof Error ? error.message : ''}`
            : request.kind === 'rename'
              ? '重命名结果尚未确认；目标可能已创建，源是否已摘除未确认。保留两个路径和原请求标识，先核对原请求；请勿换键重试。'
              : '尚未取得完整回执。原请求已保留，只能核对原请求；请勿换键重试。必要时由维护流程收口。',
        });
    } finally {
      this.shared.running.delete(key);
      this.notify();
    }
  }
}
