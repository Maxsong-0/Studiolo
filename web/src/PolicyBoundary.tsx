import { useEffect, useState } from 'react';
import { PolicyPanel } from './components/PolicyPanel';
import {
  isAbsent,
  readDesired,
  readPolicyStatus,
  readVersions,
} from './policyApi';
import type { DesiredRecord, PolicyStatus, VersionMeta } from './policyModel';

type Status =
  | { kind: 'loading' }
  | {
      kind: 'ready';
      status: PolicyStatus;
      desired: DesiredRecord[];
      versions: VersionMeta[];
    }
  | { kind: 'absent' }
  | { kind: 'error'; message: string };

/** 策略平面边界：未接入显示「未接入」而不是空白；读取失败显示原因而不是假装为空。 */
export function PolicyBoundary() {
  const [status, setStatus] = useState<Status>({ kind: 'loading' });
  useEffect(() => {
    const controller = new AbortController();
    Promise.all([
      readPolicyStatus(controller.signal),
      readDesired(controller.signal),
      readVersions(controller.signal),
    ])
      .then(([status, desired, versions]) => {
        if (!controller.signal.aborted)
          setStatus({ kind: 'ready', status, desired, versions });
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        if (isAbsent(error)) {
          setStatus({ kind: 'absent' });
          return;
        }
        setStatus({
          kind: 'error',
          message:
            error instanceof Error ? error.message : '策略平面读取失败。',
        });
      });
    return () => controller.abort();
  }, []);
  if (status.kind === 'loading') {
    return (
      <section className="policy-panel" aria-label="策略平面">
        <p>正在读取策略平面…</p>
      </section>
    );
  }
  if (status.kind === 'absent') {
    return (
      <section className="policy-panel" aria-label="策略平面">
        <p className="policy-hint">
          本入口未装配策略平面（启动时未给 --policy-data）。规则母本、skills
          与能力集的发布只能在装配了策略平面的会话入口进行。
        </p>
      </section>
    );
  }
  if (status.kind === 'error') {
    return (
      <section className="policy-panel" aria-label="策略平面">
        <p className="policy-warning" role="alert">
          读取策略平面失败：{status.message}
        </p>
      </section>
    );
  }
  return (
    <PolicyPanel
      initial={{
        status: status.status,
        desired: status.desired,
        versions: status.versions,
      }}
    />
  );
}
