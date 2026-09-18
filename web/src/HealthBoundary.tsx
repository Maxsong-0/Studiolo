import { useEffect, useState } from 'react';
import { HealthPanel } from './HealthPanel';
import { readHealth } from './healthApi';
import type { HealthReport } from './healthApi';

type Status =
  | { kind: 'loading' }
  | { kind: 'ready'; report: HealthReport }
  | { kind: 'error'; message: string };

/**
 * 健康页边界：读取失败时显示"未知"而不是空白，更不会显示成正常。
 */
export function HealthBoundary() {
  const [status, setStatus] = useState<Status>({ kind: 'loading' });
  useEffect(() => {
    const controller = new AbortController();
    readHealth(controller.signal)
      .then((report) => {
        if (!controller.signal.aborted) setStatus({ kind: 'ready', report });
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        const message =
          error instanceof Error ? error.message : '健康页读取失败。';
        setStatus({ kind: 'error', message });
      });
    return () => controller.abort();
  }, []);
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
