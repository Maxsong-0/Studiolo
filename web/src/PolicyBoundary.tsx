import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { PolicyPanel } from './components/PolicyPanel';
import { isAbsent, readPolicySnapshot } from './policyApi';
import type { ListedDesired, PolicyStatus, VersionMeta } from './policyModel';
import { Cell, EmptyState, ErrorState, GridFrame, Skeleton } from './ui';
import './styles/policy.css';

type Status =
  | { kind: 'loading' }
  | {
      kind: 'ready';
      status: PolicyStatus;
      desired: ListedDesired[];
      versions: VersionMeta[];
    }
  | { kind: 'absent' }
  | { kind: 'error'; message: string };

/** 读取中、未接入、读取失败都占满一个 12 列的格子，与就绪后的网格同一版式。 */
function StateCell({ children }: { children: ReactNode }) {
  return (
    <GridFrame className="policy-grid">
      <Cell span={12} className="policy-cell" aria-label="策略平面">
        {children}
      </Cell>
    </GridFrame>
  );
}

/**
 * 策略平面边界：未接入显示「未接入」而不是空白；读取失败显示原因而不是假装为空。
 * 失败后只在人点「重新读取」时再读一次整份快照（仍是顺序三读，见 policyApi），绝不自动重试。
 */
export function PolicyBoundary() {
  const [status, setStatus] = useState<Status>({ kind: 'loading' });
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    readPolicySnapshot(controller.signal)
      .then(({ status, desired, versions }) => {
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
  }, [attempt]);
  if (status.kind === 'loading') {
    return (
      <StateCell>
        <Skeleton label="正在读取策略平面…" lines={4} />
      </StateCell>
    );
  }
  if (status.kind === 'absent') {
    return (
      <StateCell>
        <EmptyState title="未接入策略平面">
          <p>
            本入口未装配策略平面（启动时未给 --policy-data）。规则母本、skills
            与能力集的发布只能在装配了策略平面的会话入口进行。
          </p>
        </EmptyState>
      </StateCell>
    );
  }
  if (status.kind === 'error') {
    return (
      <StateCell>
        <ErrorState
          title="读取策略平面失败"
          retryLabel="重新读取"
          onRetry={() => {
            setStatus({ kind: 'loading' });
            setAttempt((count) => count + 1);
          }}
        >
          <p>{status.message}</p>
        </ErrorState>
      </StateCell>
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
