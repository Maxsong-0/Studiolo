import { HealthOverall, HealthPanel } from './HealthPanel';
import { StatusMark } from './ui';
import { useHealthStatus } from './shell/useHealthStatus';
import type { HealthStatus } from './shell/useHealthStatus';
import './styles/health.css';

/**
 * 健康页三态展示：读取失败时显示"未知"而不是空白，更不会显示成正常。
 * 取数由调用方负责（顶栏状态胶囊在外壳里读一次），这里只管怎么显示。
 * 读取中只给眼睛看：状态浮层里别处还有播报，这里不再加 role=status。
 */
export function HealthView({ status }: { status: HealthStatus }) {
  if (status.kind === 'loading') {
    return (
      <section className="health-panel" aria-label="本机健康">
        <p className="health-loading">
          <StatusMark state="pending">正在读取本机健康证据…</StatusMark>
        </p>
      </section>
    );
  }
  if (status.kind === 'error') {
    return (
      <section className="health-panel" aria-label="本机健康">
        <HealthOverall state="unknown" />
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
