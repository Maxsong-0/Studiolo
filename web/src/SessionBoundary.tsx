import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import type { ReactNode } from 'react';
import { SessionClient } from './sessionClient';
import { installSessionReader } from './sessionHttp';
import { watchSession } from './sessionLifecycle';
import { SessionStatusContext } from './sessionStatus';
import type { SessionStatus } from './sessionStatus';
import { SessionAccount } from './components/SessionAccount';
import { Icon } from './components/Icon';
import { Button } from './ui';
import {
  installCanonicalRestorer,
  installCanonicalWriter,
} from './canonicalApi';
import { DraftBoundary, DraftScopeBoundary } from './DraftBoundary';
import {
  FileOperationBoundary,
  FileOperationScopeBoundary,
} from './FileOperationBoundary';
import { installFileOperationWriter } from './fileOperationApi';
import { installPolicyWriter } from './policyApi';
import { installFactorRuntime } from './factorApi';
import { installExecutionWriter } from './executionApi';
import './styles/session.css';

export function SessionBoundary(props: {
  children: (account: ReactNode) => ReactNode;
  client?: SessionClient;
}) {
  return (
    <FileOperationBoundary>
      <DraftBoundary>
        <SessionBoundaryContent {...props} />
      </DraftBoundary>
    </FileOperationBoundary>
  );
}

function SessionBoundaryContent({
  children,
  client: supplied,
}: {
  children: (account: ReactNode) => ReactNode;
  client?: SessionClient;
}) {
  const [client] = useState(() => supplied ?? new SessionClient());
  const state = useSyncExternalStore(client.subscribe, client.getSnapshot);
  useEffect(() => {
    const uninstall = installSessionReader(client.read);
    const uninstallWriter = installCanonicalWriter(client.replace);
    const uninstallRestorer = installCanonicalRestorer(client.restore);
    const uninstallFileOperations = installFileOperationWriter(
      client.fileOperation,
    );
    const uninstallPolicy = installPolicyWriter(client.policyPost);
    const uninstallFactor = installFactorRuntime({
      state: client.factorState,
      submit: client.factor,
    });
    const uninstallExecution = installExecutionWriter(client.executionCommand);
    const unwatch = watchSession(client);
    void client.check();
    return () => {
      unwatch();
      client.stop();
      uninstall();
      uninstallWriter();
      uninstallRestorer();
      uninstallFileOperations();
      uninstallPolicy();
      uninstallFactor();
      uninstallExecution();
    };
  }, [client]);
  useEffect(() => {
    if (state.phase !== 'ready' || !state.session) return;
    // Client time is only a recheck hint; skew must not cause a tight request loop.
    const delay = Math.max(
      30_000,
      state.session.absolute_expires_at_ms - Date.now(),
    );
    const timer = window.setTimeout(
      client.resume,
      Math.min(delay, 2_147_483_647),
    );
    return () => window.clearTimeout(timer);
  }, [client, state.phase, state.session]);
  // 顶栏状态胶囊的会话一项：只发快照里的四项，值按这四项重建，别的状态变化不打扰胶囊。
  const factorNotice = state.factorNotice ?? null;
  const status = useMemo<SessionStatus>(
    () => ({
      phase: state.phase,
      session: state.session,
      busy: state.busy,
      factorNotice,
    }),
    [state.phase, state.session, state.busy, factorNotice],
  );

  if (state.phase === 'legacy' || state.phase === 'ready') {
    return (
      <SessionStatusContext.Provider value={status}>
        <DraftScopeBoundary
          scope={state.phase === 'ready' ? (state.ownerScope ?? null) : null}
        >
          <FileOperationScopeBoundary
            scope={state.phase === 'ready' ? (state.ownerScope ?? null) : null}
          >
            <div className="session-content" key={state.revision}>
              {children(
                state.phase === 'ready' ? (
                  <SessionAccount client={client} state={state} />
                ) : null,
              )}
            </div>
          </FileOperationScopeBoundary>
        </DraftScopeBoundary>
      </SessionStatusContext.Provider>
    );
  }
  const checking = state.phase === 'checking';
  const signedOut = state.phase === 'signed-out';
  // login 与 check 都在内部接住失败，并把结果写进 state.message 由下方提示展示，不会拒绝；
  // 点击处理器只负责发起，显式丢弃返回的 Promise，保持 React 事件处理器的 void 约定。
  const proceed = () => {
    void (signedOut ? client.login() : client.check());
  };
  // 登录闸门（规范第 1、2、3 节）：纸色底上居中一个 2px 墨框格子；字标、图名、说明、一个动作。
  // 核对中是空心方块，出问题是红色方块，未登录不加标记：颜色之外总有形状与原文。
  return (
    <main className="session-gate">
      <section className="session-welcome" aria-labelledby="session-title">
        <p className="session-brand">
          Studiolo
          <span className="session-brand-dot" aria-hidden="true">
            .
          </span>
        </p>
        <div className="session-welcome-body">
          <h1 id="session-title" className="session-title">
            {signedOut
              ? '进入你的工作台'
              : checking
                ? '核对登录状态'
                : '暂时无法继续'}
          </h1>
          <p
            className="session-message"
            data-tone={checking ? 'pending' : signedOut ? 'plain' : 'alert'}
            role={checking || signedOut ? 'status' : 'alert'}
          >
            {state.message}
          </p>
          {checking ? null : (
            <Button
              variant="primary"
              className="session-primary"
              onClick={proceed}
            >
              <Icon name={signedOut ? 'arrow' : 'refresh'} />
              {signedOut ? '登录工作台' : '核对登录状态'}
            </Button>
          )}
        </div>
      </section>
    </main>
  );
}
