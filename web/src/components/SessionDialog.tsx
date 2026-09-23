import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import type { SessionClient } from '../sessionClient';
import type { RevokeScope, SessionInfo } from '../sessionModel';
import { sessionTime } from '../sessionStatus';
import { useDialogSurface } from '../useDialogSurface';
import { Button, StatusMark, Surface } from '../ui';
import type { StatusState } from '../ui';
import { Icon } from './Icon';
import { FactorPanel } from './FactorPanel';
import { useDraftCount } from '../draftContext';
import { exportDrafts } from '../draftExport';
import '../styles/session.css';
// 「退出前处理内存草稿」借用编辑器的不可逆确认框（.editor-leave，红色虚线框）。
import '../styles/editor.css';

const states: Record<SessionInfo['state'], string> = {
  pending: '建立中',
  active: '有效',
  revoked: '已退出',
  expired: '已过期',
  abandoned: '未启用',
};

/** 会话状态的方块：有效实心、建立中空心，已结束的一律休眠虚线框——结束不是故障，不用红。 */
const marks: Record<SessionInfo['state'], StatusState> = {
  pending: 'pending',
  active: 'ok',
  revoked: 'dormant',
  expired: 'dormant',
  abandoned: 'dormant',
};

/**
 * 等宽会话时间（本机时区 YYYY-MM-DD HH:mm）。超出 Date 范围的毫秒值显示「—」，
 * 原因写进 title 并给读屏一份，绝不补一个时间。
 */
export function SessionTime({ ms }: { ms: number }) {
  const time = sessionTime(ms);
  if (!time)
    return (
      <span className="session-time" title="时间不可用">
        —<span className="sr-only">时间不可用</span>
      </span>
    );
  return (
    <time className="session-time" dateTime={time.iso}>
      {time.text}
    </time>
  );
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
  const dismissAccount = useCallback(() => {
    secretCleanup.current?.();
    dismiss();
  }, [dismiss]);
  // Esc 走 dialog 的 cancel：拦下浏览器的直接关闭，先清掉未提交的因子输入再收起。
  // 用原生监听而不是 onCancel 属性：Surface 的属性类型只收通用 HTML 属性；紧跟 showModal 挂上。
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    function cancel(event: Event) {
      event.preventDefault();
      dismissAccount();
    }
    element.addEventListener('cancel', cancel);
    return () => element.removeEventListener('cancel', cancel);
  }, [dismissAccount]);
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
    <Surface
      as="dialog"
      ref={ref}
      appear={false}
      className="session-dialog"
      aria-labelledby="account-title"
    >
      <header className="session-dialog-header">
        <div>
          <h2 id="account-title">账户与会话</h2>
          <p>查看登录记录，退出不再使用的会话。</p>
        </div>
        <button
          type="button"
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
            <Button size="sm" onClick={() => exportDrafts(drafts.list())}>
              导出全部草稿与恢复信息
            </Button>
            <Button size="sm" onClick={() => setPendingExit(null)}>
              返回并保留草稿
            </Button>
            <Button
              variant="danger"
              size="sm"
              onClick={() => void revoke(pendingExit, true)}
            >
              放弃内存草稿并退出
            </Button>
          </section>
        ) : null}
        <section className="session-current" aria-label="当前登录">
          <div className="session-section-head">
            <h3>当前登录</h3>
            <Button
              size="sm"
              disabled={busy}
              onClick={() => void revoke({ scope: 'current' })}
            >
              <Icon name="logout" />
              退出登录
            </Button>
          </div>
          {/* 图纸标题栏的小表：等宽标签在上、读数在下，格子之间 1px 墨线。 */}
          <dl className="session-facts">
            <div>
              <dt>状态</dt>
              <dd>
                <StatusMark state={marks[current.state]}>
                  {states[current.state]}
                </StatusMark>
              </dd>
            </div>
            <div>
              <dt>登录于</dt>
              <dd>
                <SessionTime ms={current.created_at_ms} />
              </dd>
            </div>
            <div>
              <dt>到期时间</dt>
              <dd>
                <SessionTime ms={current.absolute_expires_at_ms} />
              </dd>
            </div>
          </dl>
        </section>
        <FactorPanel client={client} busy={busy} cleanup={secretCleanup} />
        <section className="session-records">
          <div className="session-section-head">
            <h3>会话记录</h3>
            <Button
              variant="text"
              size="sm"
              disabled={loading || busy}
              onClick={() => void load()}
            >
              核对列表
            </Button>
          </div>
          {notice ? (
            <p className="session-note" role="status">
              {notice}
            </p>
          ) : null}
          {loading ? (
            <p className="session-note" role="status">
              正在读取会话记录…
            </p>
          ) : null}
          {error ? (
            <p className="session-error" role="alert">
              {error}
            </p>
          ) : null}
          {sessions ? (
            <ul className="session-list" aria-label="会话记录">
              {sessions.map((session, index) => (
                <li key={session.id}>
                  <div className="session-row">
                    <h4>
                      {session.id === current.id
                        ? '此浏览器'
                        : `会话 ${index + 1}`}
                    </h4>
                    <StatusMark
                      state={marks[session.state]}
                      className="session-state"
                    >
                      {states[session.state]}
                    </StatusMark>
                  </div>
                  <p className="session-row-times">
                    <span>
                      建立于 <SessionTime ms={session.created_at_ms} />
                    </span>
                    <span>
                      最近活动 <SessionTime ms={session.last_seen_at_ms} />
                    </span>
                  </p>
                  {session.state === 'active' || session.state === 'pending' ? (
                    <Button
                      variant="danger"
                      size="sm"
                      className="session-revoke"
                      disabled={busy || loading}
                      aria-label={
                        session.id === current.id
                          ? '退出此浏览器'
                          : `退出会话 ${index + 1}`
                      }
                      onClick={() =>
                        void revoke({
                          scope: 'session',
                          session_id: session.id,
                        })
                      }
                    >
                      <Icon name="logout" />
                      退出
                    </Button>
                  ) : null}
                </li>
              ))}
            </ul>
          ) : null}
          {sessions?.length === 0 ? (
            <p className="session-note">没有会话记录。</p>
          ) : null}
        </section>
      </div>
      <footer className="session-dialog-footer">
        <p>退出全部会话后，需要重新登录。</p>
        <Button
          variant="danger"
          disabled={busy}
          onClick={() => void revoke({ scope: 'all' })}
        >
          退出全部会话
        </Button>
      </footer>
    </Surface>
  );
}
