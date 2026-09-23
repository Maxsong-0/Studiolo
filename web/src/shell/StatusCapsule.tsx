import { useEffect, useId, useRef } from 'react';
import { Diagnostics } from '../components/Diagnostics';
import { FreshnessNotice } from '../components/FreshnessNotice';
import { Icon } from '../components/Icon';
import { SessionTime } from '../components/SessionDialog';
import type { Freshness } from '../freshness';
import { HealthView } from '../HealthBoundary';
import { SESSION_ACCOUNT_ATTRIBUTE, useSessionStatus } from '../sessionStatus';
import type { SessionStatus } from '../sessionStatus';
import { Button, StatusMark } from '../ui';
import type { StatusState } from '../ui';
import { summarizeStatus } from './statusModel';
import type { HealthStatus } from './useHealthStatus';
import './shell.css';
import '../styles/session.css';

interface Snapshot {
  sourceLabel: string;
  generation: number;
  freshness: Freshness | null | undefined;
}

/**
 * 会话一栏的阶段一行：方块＋中文，只说登录本身（已登录、核对中、会话失效…）。
 * 安全验证提示另起一个红色虚线框；胶囊上报哪一项、多严重，以 statusModel 的 sessionSignal 为准。
 */
function sessionPhase(status: SessionStatus): {
  mark: StatusState;
  text: string;
} {
  switch (status.phase) {
    case 'legacy':
      return { mark: 'ok', text: '本入口未启用会话登录' };
    case 'uncertain':
    case 'problem':
      return { mark: 'alert', text: '会话未确认' };
    case 'checking':
      return { mark: 'pending', text: '核对中' };
    case 'signed-out':
      return { mark: 'dormant', text: '未登录' };
    case 'ready':
      if (!status.session || status.session.state !== 'active')
        return { mark: 'alert', text: '会话失效' };
      return status.busy
        ? { mark: 'pending', text: '核对中' }
        : { mark: 'ok', text: '已登录' };
  }
}

/** 把焦点交给顶栏的账户入口；对话框只从那里开，胶囊不另开一份。 */
function focusAccountEntry() {
  document
    .querySelector<HTMLElement>(`[${SESSION_ACCOUNT_ATTRIBUTE}]`)
    ?.focus();
}

/**
 * 会话一栏：阶段、会话 ID 前 8 位、登录与到期时间（本机时区，等宽）、安全验证提示原文，
 * 以及去顶栏账户入口的带路按钮。拿不到会话信息时直说「会话信息不可用」，不当作正常。
 */
function SessionBlock({ status }: { status: SessionStatus | null }) {
  const hintId = useId();
  if (status === null)
    return (
      <div className="status-session">
        <p className="status-session-phase">
          <StatusMark state="alert">会话信息不可用</StatusMark>
        </p>
        <p className="status-session-hint">
          本页没有从登录入口拿到会话读数，不能确认登录状态。
        </p>
      </div>
    );
  const phase = sessionPhase(status);
  const session = status.session;
  return (
    <div className="status-session">
      <p className="status-session-phase">
        <StatusMark state={phase.mark}>{phase.text}</StatusMark>
      </p>
      {session ? (
        <dl className="status-session-facts">
          <div>
            <dt>会话 ID</dt> <dd>{session.id.slice(0, 8)}</dd>
          </div>
          <div>
            <dt>登录于</dt>{' '}
            <dd>
              <SessionTime ms={session.created_at_ms} />
            </dd>
          </div>
          <div>
            <dt>到期</dt>{' '}
            <dd>
              <SessionTime ms={session.absolute_expires_at_ms} />
            </dd>
          </div>
        </dl>
      ) : null}
      {status.factorNotice ? (
        <p className="status-session-notice" role="status">
          {status.factorNotice}
        </p>
      ) : null}
      {status.phase === 'ready' ? (
        <p className="status-session-entry">
          <Button
            variant="text"
            size="sm"
            className="status-session-link"
            aria-describedby={hintId}
            onClick={focusAccountEntry}
          >
            前往账户与会话
          </Button>
          <span id={hintId}>从顶栏账户入口打开</span>
        </p>
      ) : null}
    </div>
  );
}

/**
 * 顶栏状态胶囊：把内容新鲜度、本机健康、会话与数据提示收成一句；点开是浮层，逐项列出原文与动作。
 * 浮层不是模态：Esc 收起并把焦点还给胶囊，点外部或用键盘移出也收起。
 * 会话读数来自 SessionBoundary 发下的 context；外面没有 SessionBoundary 时按「会话未知」处理。
 */
export function StatusCapsule({
  open,
  onOpenChange,
  snapshot,
  refreshing,
  failed,
  onVerify,
  onRefresh,
  health,
  diagnostics,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 工作台快照；null 表示还没读到。 */
  snapshot: Snapshot | null;
  /** 工作台正在读取或刷新。 */
  refreshing: boolean;
  /** 最近一次读取工作台失败。 */
  failed: boolean;
  /** 「重新核验」：只重新核验内容新鲜度（刷新工作台快照）。 */
  onVerify: () => void;
  /** 「刷新快照」：重读工作台快照与本机健康。 */
  onRefresh: () => void;
  health: HealthStatus;
  diagnostics: string[];
}) {
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const popover = useRef<HTMLElement>(null);
  const popoverId = useId();
  const titleId = useId();
  const session = useSessionStatus();
  const summary = summarizeStatus({
    loaded: snapshot !== null,
    freshness: snapshot?.freshness,
    refreshing,
    failed,
    health,
    session,
    diagnostics: diagnostics.length,
  });

  useEffect(() => {
    if (!open) return;
    function outside(event: PointerEvent) {
      if (!root.current?.contains(event.target as Node)) onOpenChange(false);
    }
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open, onOpenChange]);

  // 从别处打开（如驾驶舱「查看数据提示」）时把焦点带进浮层；从胶囊本身打开时焦点留在胶囊上。
  useEffect(() => {
    if (open && !root.current?.contains(document.activeElement))
      popover.current?.focus();
  }, [open]);

  return (
    <div
      ref={root}
      className="shell-status"
      onKeyDown={(event) => {
        if (event.key !== 'Escape' || !open || event.nativeEvent.isComposing)
          return;
        event.preventDefault();
        onOpenChange(false);
        trigger.current?.focus();
      }}
      onBlur={(event) => {
        // 键盘把焦点移出胶囊与浮层才收起；点到浮层里的文字（relatedTarget 为空）不算移出。
        const next = event.relatedTarget;
        if (open && next instanceof Node && !root.current?.contains(next))
          onOpenChange(false);
      }}
    >
      <button
        ref={trigger}
        type="button"
        className="status-capsule"
        data-level={summary.level}
        aria-expanded={open}
        aria-controls={open ? popoverId : undefined}
        onClick={() => onOpenChange(!open)}
      >
        <span className="sr-only">工作台状态：</span>
        {summary.level === 'problem' ? (
          <>
            <span aria-hidden="true">[</span>
            <span className="status-capsule-label">{summary.label}</span>
            <span aria-hidden="true">]</span>
          </>
        ) : (
          <>
            <span className="status-capsule-mark" aria-hidden="true">
              {summary.level === 'normal' ? '■' : '□'}
            </span>
            <span className="status-capsule-label">{summary.label}</span>
          </>
        )}
      </button>
      {open ? (
        <section
          ref={popover}
          id={popoverId}
          className="status-popover"
          aria-labelledby={titleId}
          tabIndex={-1}
        >
          <header className="status-popover-head">
            <h2 id={titleId}>工作台状态</h2>
            <Button
              className="refresh-button"
              aria-disabled={refreshing}
              onClick={() => {
                if (!refreshing) onRefresh();
              }}
            >
              <Icon name="refresh" />
              {refreshing && snapshot ? '刷新中' : '刷新快照'}
            </Button>
          </header>
          <div className="status-block">
            <h3 className="status-block-title">新鲜度</h3>
            <p className="status-source">
              <span>来源</span>
              <strong>
                {snapshot?.sourceLabel ?? (refreshing ? '读取中' : '未连接')}
              </strong>
              {snapshot ? <span>代次 {snapshot.generation}</span> : null}
            </p>
            {snapshot ? (
              <FreshnessNotice
                freshness={snapshot.freshness}
                refreshing={refreshing}
                failed={failed}
                onRefresh={onVerify}
              />
            ) : null}
          </div>
          <div className="status-block">
            <h3 className="status-block-title">本机健康</h3>
            <HealthView status={health} />
          </div>
          <div className="status-block">
            <h3 className="status-block-title">会话</h3>
            <SessionBlock status={session} />
          </div>
          <div className="status-block">
            <h3 className="status-block-title">
              数据提示{diagnostics.length ? `（${diagnostics.length}）` : ''}
            </h3>
            <Diagnostics messages={diagnostics} />
          </div>
        </section>
      ) : null}
    </div>
  );
}
