import { freshnessReason } from '../freshness';
import type { Freshness } from '../freshness';

interface Props {
  freshness: Freshness | null | undefined;
  refreshing: boolean;
  failed: boolean;
  onRefresh: () => void;
}

export function FreshnessNotice({
  freshness,
  refreshing,
  failed,
  onRefresh,
}: Props) {
  const current = !refreshing && !failed;
  const title = refreshing
    ? '正在重新核验'
    : failed
      ? '本次核验失败'
      : !freshness
        ? '新鲜度未知'
        : freshness.stale
          ? '快照已陈旧'
          : '已追平已知提交';
  const description = !current
    ? '上次快照仍可阅读；重新核验成功前，不能确认当前新鲜度。'
    : !freshness
      ? '当前服务未提供 canonical 新鲜度证据，不能据此启用编辑。'
      : freshness.stale
        ? freshnessReason(freshness)
        : '快照发布时已追平本写者的已知提交，目录监视正常。';

  return (
    <section className="freshness-notice" aria-label="内容新鲜度">
      <div
        className="freshness-notice-copy"
        aria-live="polite"
        aria-atomic="true"
      >
        <strong>{title}</strong>
        <p>{description}</p>
        {current && freshness ? (
          <p className="freshness-watermarks">
            索引水位 {freshness.indexed_through ?? '未知'} · 提交水位{' '}
            {freshness.committed_through ?? '未知'}
            {' · '}未决写入
            {freshness.unresolved === null
              ? '未知'
              : freshness.unresolved
                ? '存在'
                : '无'}
          </p>
        ) : null}
        <p className="freshness-scope">
          这是快照观测；文档编辑还需独立授权与保存时的版本核对。
        </p>
      </div>
      <button
        className="outline-button"
        aria-disabled={refreshing}
        onClick={() => {
          if (!refreshing) onRefresh();
        }}
      >
        重新核验
      </button>
    </section>
  );
}
