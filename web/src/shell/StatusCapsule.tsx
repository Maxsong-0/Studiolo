import { useEffect, useId, useRef } from 'react';
import { Diagnostics } from '../components/Diagnostics';
import { FreshnessNotice } from '../components/FreshnessNotice';
import { Icon } from '../components/Icon';
import type { Freshness } from '../freshness';
import { HealthView } from '../HealthBoundary';
import { summarizeStatus } from './statusModel';
import type { HealthStatus } from './useHealthStatus';
import './shell.css';

interface Snapshot {
  sourceLabel: string;
  generation: number;
  freshness: Freshness | null | undefined;
}

/**
 * 顶栏状态胶囊：把内容新鲜度、本机健康与数据提示收成一句；点开是浮层，逐项列出原文与动作。
 * 浮层不是模态：Esc 收起并把焦点还给胶囊，点外部或用键盘移出也收起。
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
  const summary = summarizeStatus({
    loaded: snapshot !== null,
    freshness: snapshot?.freshness,
    refreshing,
    failed,
    health,
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
            <button
              type="button"
              className="outline-button refresh-button"
              aria-disabled={refreshing}
              onClick={() => {
                if (!refreshing) onRefresh();
              }}
            >
              <Icon name="refresh" />
              {refreshing && snapshot ? '刷新中' : '刷新快照'}
            </button>
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
