import { freshnessReason } from '../freshness';
import type { Freshness } from '../freshness';
import { Button, StatusMark } from '../ui';
import type { StatusState } from '../ui';
import '../styles/freshness.css';

interface Props {
  freshness: Freshness | null | undefined;
  refreshing: boolean;
  failed: boolean;
  onRefresh: () => void;
}

/**
 * 内容新鲜度（状态浮层的「新鲜度」一栏）：状态行＋说明＋等宽水位清单＋「重新核验」。
 * 状态行的方块：已追平实心、核验在途空心、陈旧或失败红色、没有证据休眠虚线——未知不冒充正常。
 */
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
  const mark: StatusState = refreshing
    ? 'pending'
    : failed
      ? 'alert'
      : !freshness
        ? 'dormant'
        : freshness.stale
          ? 'alert'
          : 'ok';
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
        <p className="freshness-state">
          <StatusMark state={mark}>{title}</StatusMark>
        </p>
        <p className="freshness-description">{description}</p>
        {current && freshness ? (
          // 标签与读数之间留一个空格：读屏与纯文本里读成「提交水位 未知」，不粘成一串。
          <dl className="freshness-watermarks">
            <div>
              <dt>索引水位</dt> <dd>{freshness.indexed_through ?? '未知'}</dd>
            </div>
            <div>
              <dt>提交水位</dt> <dd>{freshness.committed_through ?? '未知'}</dd>
            </div>
            <div>
              <dt>未决写入</dt>{' '}
              <dd>
                {freshness.unresolved === null
                  ? '未知'
                  : freshness.unresolved
                    ? '存在'
                    : '无'}
              </dd>
            </div>
          </dl>
        ) : null}
        <p className="freshness-scope">
          这是快照观测；文档编辑还需独立授权与保存时的版本核对。
        </p>
      </div>
      <Button
        className="freshness-verify"
        aria-disabled={refreshing}
        onClick={() => {
          if (!refreshing) onRefresh();
        }}
      >
        重新核验
      </Button>
    </section>
  );
}
