/** Canonical 发布观测不等于目标文档 revision，也不单独授予编辑权限。 */
export interface Freshness {
  stale: boolean;
  indexed_through: string | null;
  committed_through: string | null;
  unresolved: boolean | null;
  watcher_healthy: boolean;
  reason: string | null;
}

const reasons: Record<string, string> = {
  initializing: '服务正在准备首次完整快照。',
  refreshing: '服务正在重新核验内容。',
  periodic_check: '服务正在执行周期核验。',
  changed: '已观察到内容变化，快照尚未重新核验。',
  watcher_rescan: '目录事件不完整，需要重新扫描。',
  watcher_failed: '目录监视不可用，需要恢复监视并重新扫描。',
  capture_failed: '内容读取未完成。',
  build_failed: '新快照构建失败。',
  binding_changed: '批准的目录绑定发生变化，需要维护核对。',
  unresolved: '存在结果未决的写入，需要先由维护流程处理。',
  writer_state_invalid: '写者状态无法确认，需要维护核对。',
  stopping: '服务正在停止。',
  unavailable: '服务暂时无法确认内容状态。',
};

const watermark = (value: unknown): boolean =>
  value === null ||
  (typeof value === 'string' &&
    /^(0|[1-9][0-9]{0,19})$/.test(value) &&
    (value.length < 20 || value <= '18446744073709551615'));

export function validFreshness(value: unknown): value is Freshness | null {
  if (value === null) return true;
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const data = value as Record<string, unknown>;
  if (
    typeof data.stale !== 'boolean' ||
    typeof data.watcher_healthy !== 'boolean' ||
    !(data.unresolved === null || typeof data.unresolved === 'boolean') ||
    !watermark(data.indexed_through) ||
    !watermark(data.committed_through) ||
    !(
      data.reason === null ||
      (typeof data.reason === 'string' &&
        /^[a-z][a-z0-9_]{0,63}$/.test(data.reason))
    )
  )
    return false;
  return (
    data.stale ||
    (data.watcher_healthy &&
      data.unresolved === false &&
      data.reason === null &&
      data.indexed_through !== null &&
      data.indexed_through === data.committed_through)
  );
}

export function freshnessReason(freshness: Freshness): string {
  if (freshness.reason) {
    return Object.hasOwn(reasons, freshness.reason)
      ? reasons[freshness.reason]
      : `服务端报告了新的核验原因：${freshness.reason}。`;
  }
  if (freshness.unresolved) return reasons.unresolved;
  if (!freshness.watcher_healthy) return reasons.watcher_failed;
  return '当前快照尚未追平已知提交。';
}
