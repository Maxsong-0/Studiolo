import { useCallback, useEffect, useRef, useState } from 'react';
import type { SessionClient } from '../sessionClient';
import type { RevokeScope, SessionInfo } from '../sessionModel';
import { useDialogSurface } from '../useDialogSurface';
import { Icon } from './Icon';
import { FactorPanel } from './FactorPanel';
import { useDraftCount } from '../draftContext';
import { exportDrafts } from '../draftExport';

const states = {
  pending: '建立中',
  active: '有效',
  revoked: '已退出',
  expired: '已过期',
  abandoned: '未启用',
};
function date(value: number) {
  const formatted = new Date(value);
  return Number.isNaN(formatted.getTime())
    ? '时间不可用'
    : formatted.toLocaleString();
}

export function SessionDialog({
  client,
  current,
  busy,
  onClose,
}: {
  client: SessionClient;
  current: SessionInfo;
  busy: boolean;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const { dismiss } = useDialogSurface(ref, onClose);
  const secretCleanup = useRef<(() => void) | null>(null);
  function dismissAccount() {
    secretCleanup.current?.();
    dismiss();
  }
  const [sessions, setSessions] = useState<SessionInfo[] | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const [notice, setNotice] = useState('');
  const { store: drafts, count: draftCount } = useDraftCount();
  const [pendingExit, setPendingExit] = useState<RevokeScope | null>(null);
  const request = useRef<AbortController | null>(null);
  const load = useCallback(async () => {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    setLoading(true);
    setError('');
    try {
      const list = await client.list(controller.signal);
      if (!controller.signal.aborted) setSessions(list);
    } catch {
      if (!controller.signal.aborted)
        setError('暂时无法读取会话列表，请重新核对。');
    } finally {
      if (!controller.signal.aborted) setLoading(false);
    }
  }, [client]);
  useEffect(() => {
    void load();
    return () => request.current?.abort();
  }, [load]);
  async function revoke(scope: RevokeScope, discard = false) {
    const own = scope.scope !== 'session' || scope.session_id === current.id;
    if (own && draftCount && !discard) {
      setPendingExit(scope);
      return;
    }
    if (own && discard) drafts.clear();
    setPendingExit(null);
    setNotice('');
    if (await client.revoke(scope)) {
      if (!ref.current?.isConnected) return;
      if (scope.scope === 'session' && scope.session_id !== current.id) {
        setNotice('已退出所选会话。');
        void load();
      }
    }
  }
  return (
    <dialog
      className="session-dialog"
      ref={ref}
      aria-labelledby="account-title"
      onCancel={(event) => {
        event.preventDefault();
        dismissAccount();
      }}
    >
      <header className="session-dialog-header">
        <div>
          <h2 id="account-title">账户与会话</h2>
          <p>查看登录记录，退出不再使用的会话。</p>
        </div>
        <button
          className="icon-button"
          onClick={dismissAccount}
          data-dialog-dismiss
          aria-label="关闭账户与会话"
        >
          <Icon name="close" />
        </button>
      </header>
      <div className="session-dialog-body">
        {pendingExit ? (
          <section
            className="editor-leave"
            role="alertdialog"
            aria-label="退出前处理草稿"
          >
            <h3>退出前处理内存草稿</h3>
            <p>共有 {draftCount} 篇草稿。请先导出，或明确放弃后退出。</p>
            <button
              className="outline-button"
              onClick={() => exportDrafts(drafts.list())}
            >
              导出全部草稿与恢复信息
            </button>
            <button
              className="outline-button"
              onClick={() => setPendingExit(null)}
            >
              返回并保留草稿
            </button>
            <button
              className="text-button"
              onClick={() => void revoke(pendingExit, true)}
            >
              放弃内存草稿并退出
            </button>
          </section>
        ) : null}
        <section className="session-current" aria-label="当前登录">
          <Icon name="account" />
          <div>
            <h3>当前登录</h3>
            <p>到期时间 {date(current.absolute_expires_at_ms)}</p>
          </div>
          <button
            className="outline-button"
            disabled={busy}
            onClick={() => void revoke({ scope: 'current' })}
          >
            <Icon name="logout" />
            退出登录
          </button>
        </section>
        <FactorPanel client={client} busy={busy} cleanup={secretCleanup} />
        <div className="session-list-heading">
          <h3>会话记录</h3>
          <button
            className="text-button"
            disabled={loading || busy}
            onClick={() => void load()}
          >
            核对列表
          </button>
        </div>
        {notice ? <p role="status">{notice}</p> : null}
        {loading ? <p role="status">正在读取会话记录…</p> : null}
        {error ? <p role="alert">{error}</p> : null}
        {sessions ? (
          <ul className="session-list" aria-label="会话记录">
            {sessions.map((session, index) => (
              <li key={session.id}>
                <div>
                  <h4>
                    {session.id === current.id
                      ? '此浏览器'
                      : `会话 ${index + 1}`}
                    <span className="session-state">
                      {states[session.state]}
                    </span>
                  </h4>
                  <p>建立于 {date(session.created_at_ms)}</p>
                  <p>最近活动 {date(session.last_seen_at_ms)}</p>
                </div>
                {session.state === 'active' || session.state === 'pending' ? (
                  <button
                    className="outline-button"
                    disabled={busy || loading}
                    aria-label={
                      session.id === current.id
                        ? '退出此浏览器'
                        : `退出会话 ${index + 1}`
                    }
                    onClick={() =>
                      void revoke({ scope: 'session', session_id: session.id })
                    }
                  >
                    <Icon name="logout" />
                    退出
                  </button>
                ) : null}
              </li>
            ))}
          </ul>
        ) : null}
        {sessions?.length === 0 ? <p>没有会话记录。</p> : null}
      </div>
      <footer className="session-dialog-footer">
        <p>退出全部会话后，需要重新登录。</p>
        <button
          className="outline-button session-danger"
          disabled={busy}
          onClick={() => void revoke({ scope: 'all' })}
        >
          退出全部会话
        </button>
      </footer>
    </dialog>
  );
}
