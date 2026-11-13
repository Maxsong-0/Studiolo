import { Button } from '../ui';
import { cx } from '../ui/dom';
import { Ago } from './Ago';
import { ATTENTION_IDLE_MS } from './attention';
import { AUTO_REFRESH_INTERVAL_MS } from './autoRefresh';
import { useAutoRefreshStatus } from './useAutoRefresh';
import '../styles/live.css';

const RULE = `每 ${AUTO_REFRESH_INTERVAL_MS / 1000} 秒自动重读一次。页面切走、开着弹层，或者 ${
  ATTENTION_IDLE_MS / 60_000
} 分钟没有操作时先停着，回来立刻补读。`;

/**
 * 这一页的数是多久以前读的（T-196）：「读于 N 秒前」，自己走字。
 * 自动刷新没读成时在同一处写明原因并给「现在重读」，数据留着上一次的；有一会儿没人操作而停着时也写明。
 * 不在 App 里（没有调度器）或一次都没读成过时不显示。
 */
export function LiveStamp({ className }: { className?: string }) {
  const { readAt, failure, idle, running, runNow } = useAutoRefreshStatus();
  if (readAt === null) return null;
  return (
    <span className={cx('live-stamp', className)}>
      <span title={RULE}>
        读于 <Ago at={readAt} />
      </span>
      {failure !== null ? (
        <span className="live-stamp__note" role="status">
          自动刷新没成功：{failure}
          <Button
            variant="text"
            size="sm"
            className="live-stamp__retry"
            disabled={running}
            onClick={runNow}
          >
            现在重读
          </Button>
        </span>
      ) : idle ? (
        <span className="live-stamp__note">有一会儿没操作，自动刷新先停着</span>
      ) : null}
    </span>
  );
}
