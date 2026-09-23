import { HealthPanel } from './HealthPanel';
import { useHealthStatus } from './shell/useHealthStatus';
import type { HealthStatus } from './shell/useHealthStatus';

/**
 * 健康页三态展示：读取失败时显示"未知"而不是空白，更不会显示成正常。
 * 取数由调用方负责（顶栏状态胶囊在外壳里读一次），这里只管怎么显示。
 */
export function HealthView({ status }: { status: HealthStatus }) {
  if (status.kind === 'loading') {
    return (
      <section className="health-panel" aria-label="本机健康">
        <p>正在读取本机健康证据…</p>
      </section>
    );
  }
  if (status.kind === 'error') {
    return (
      <section className="health-panel" aria-label="本机健康">
        <p className="health-overall health-unknown">
          <span>整体状态</span>
          <strong data-state="unknown">未知</strong>
        </p>
        <p className="health-incomplete">
          读取健康证据失败：{status.message}。未知不等于正常。
        </p>
      </section>
    );
  }
  return <HealthPanel report={status.report} />;
}

/** 自己取数的健康页边界：挂载时读取一次。 */
export function HealthBoundary() {
  const { status } = useHealthStatus();
  return <HealthView status={status} />;
}
