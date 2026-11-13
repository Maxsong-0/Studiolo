import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { boardOverviewDisabled, fetchBoardOverview } from '../boardOverviewApi';
import type { BoardOverview } from '../boardOverviewModel';
import { RevokeForm } from '../components/PolicyForms';
import { ActionPreview } from '../policy/ActionPreview';
import { PolicyFeedback } from '../policy/PolicyFeedback';
import { displayTime } from '../policy/labels';
import { REVOKE_PREVIEW, type PolicyActions } from '../policy/usePolicyActions';
import { Button, Cell, CellHeader, GridFrame } from '../ui';
import { useViewLocation } from '../useViewLocation';
import { agentsAbsent, readAgents } from './api';
import {
  DENIAL_LABEL,
  relativeCall,
  visibleInstances,
  type AgentsSnapshot,
} from './model';
import '../styles/agents.css';

type Claims =
  { kind: 'ready'; data: BoardOverview } | { kind: 'absent' | 'unavailable' };
type Loaded =
  | { kind: 'ready'; data: AgentsSnapshot; claims: Claims }
  | { kind: 'loading' | 'absent' | 'failed' };
type Target = { level: 'type' | 'instance'; target: string };
export function AgentsPanel({
  revision,
  actions,
  onRestore,
}: {
  revision: number;
  actions: PolicyActions;
  onRestore: (name: string, version: number) => void;
}) {
  const [loaded, setLoaded] = useState<Loaded>({ kind: 'loading' });
  const [retry, setRetry] = useState(0);
  const [recent, setRecent] = useState(true);
  const [target, setTarget] = useState<Target | null>(null);
  const { location, goTo } = useViewLocation();
  const selected = location.item;
  const focused = useRef<string | null>(null);
  const rowRefs = useRef(new Map<string, HTMLElement>());
  const formRef = useRef<HTMLElement>(null);
  const id = useId(),
    previewId = `${id}-preview`;
  const focusRow = useCallback(() => {
    if (!selected || focused.current === selected) return;
    const element = rowRefs.current.get(selected);
    if (!element) return;
    focused.current = selected;
    element.scrollIntoView({ block: 'nearest' });
    if (!document.querySelector('dialog[open]'))
      element.focus({ preventScroll: true });
  }, [selected]);
  useEffect(() => {
    const controller = new AbortController();
    async function load() {
      try {
        const data = await readAgents(controller.signal);
        let claims: Claims;
        try {
          claims = {
            kind: 'ready',
            data: await fetchBoardOverview(24, controller.signal),
          };
        } catch (error) {
          claims = {
            kind: boardOverviewDisabled(error) ? 'absent' : 'unavailable',
          };
        }
        if (!controller.signal.aborted) {
          setLoaded({ kind: 'ready', data, claims });
          setTarget(null);
        }
      } catch (error) {
        if (!controller.signal.aborted)
          setLoaded({ kind: agentsAbsent(error) ? 'absent' : 'failed' });
      }
    }
    void load();
    return () => controller.abort();
  }, [revision, retry]);
  useEffect(focusRow, [focusRow, loaded]);
  useEffect(() => {
    if (target) formRef.current?.focus();
  }, [target]);
  function choose(value: Target) {
    if (value.level === 'instance')
      goTo({ view: 'policy', panel: 'agents', item: value.target });
    actions.closePreview();
    setTarget(value);
  }
  function refresh() {
    actions.closePreview();
    setLoaded({ kind: 'loading' });
    setRetry((n) => n + 1);
  }
  function openInstance(instance: string) {
    focused.current = null;
    goTo({ view: 'policy', panel: 'agents', item: instance });
  }
  if (loaded.kind !== 'ready')
    return (
      <section className="agents-state" aria-live="polite">
        <p>
          {loaded.kind === 'loading'
            ? '正在读取接入方…'
            : loaded.kind === 'absent'
              ? '本入口尚未接入身份列表。'
              : '接入方身份或策略暂不可用，请稍后重试。'}
        </p>
        {loaded.kind !== 'loading' ? (
          <Button variant="secondary" onClick={refresh}>
            重新读取
          </Button>
        ) : null}
      </section>
    );
  const { data, claims } = loaded;
  const visible = visibleInstances(data, recent, selected);
  const preview =
    actions.preview?.key === REVOKE_PREVIEW &&
    actions.preview.kind === 'revoke' &&
    actions.preview.origin === 'agents'
      ? actions.preview
      : null;
  return (
    <GridFrame className="policy-grid agents">
      <Cell span={12} className="policy-cell">
        <CellHeader
          title="接入方"
          meta={
            <Button
              variant="secondary"
              size="sm"
              disabled={actions.busy}
              onClick={refresh}
            >
              刷新列表
            </Button>
          }
        />
        <p className="policy-note">
          最近一次授权调用不代表持续连接。首次调用前，时间取登记时间。
        </p>
        <p className="policy-note">
          快照 {displayTime(data.generated_at)} · 策略{' '}
          <code title={data.effective_hash}>
            {data.effective_hash.slice(0, 12)}
          </code>
        </p>
        {data.global_revoked ? (
          <p className="policy-bar__alert">
            全局已撤销接入；可在「现状」完成二次认证后解除。
          </p>
        ) : null}
        {!data.children_enabled ? (
          <p className="policy-hint">身份库尚未启用子实例，父实例均为空。</p>
        ) : null}
        <label className="agents-filter">
          <input
            type="checkbox"
            checked={recent}
            onChange={(e) => setRecent(e.target.checked)}
          />
          仅显示最近 24 小时有授权调用或登记的实例
        </label>
        {data.instances_truncated ? (
          <p className="policy-hint">
            仅列出最近登记的 500 个实例；筛选范围限于这份列表。
          </p>
        ) : null}
        {selected && !data.instances.some((row) => row.id === selected) ? (
          <p role="status">
            链接指向的实例不在当前列表中，可能已超出最近 500 条范围。
          </p>
        ) : null}
        <PolicyFeedback
          origin="agents"
          notice={actions.notice}
          error={actions.error}
        />
        {data.types.length === 0 ? (
          <p className="policy-empty">尚未登记接入类型。</p>
        ) : null}
        {data.types.map((kind) => (
          <section
            className="agents-type"
            key={kind.name}
            aria-label={`${kind.name} 接入方`}
          >
            <header className="agents-type__header">
              <div>
                <h3>{kind.name}</h3>
                <p className="policy-hint">
                  签发 {displayTime(kind.issued_at)} ·{' '}
                  {kind.capability_version === null
                    ? '尚无生效能力集'
                    : `能力集 v${kind.capability_version}`}
                </p>
                {kind.identity_revoked_at ? (
                  <p>
                    类型凭据已吊销：{displayTime(kind.identity_revoked_at)}
                    ；须由身份管理员重新签发。
                  </p>
                ) : null}
                {kind.policy_revocation ? (
                  <p>类型已撤销接入：{kind.policy_revocation.reason}</p>
                ) : null}
              </div>
              <div className="policy-actions">
                <Button
                  variant="danger"
                  size="sm"
                  disabled={actions.busy || kind.policy_revocation !== null}
                  onClick={() => choose({ level: 'type', target: kind.name })}
                >
                  撤销此类型
                </Button>
                {kind.policy_revocation && kind.capability_version !== null ? (
                  <Button
                    variant="secondary"
                    size="sm"
                    disabled={actions.busy}
                    onClick={() =>
                      onRestore(kind.name, kind.capability_version!)
                    }
                  >
                    打开能力集草案
                  </Button>
                ) : null}
              </div>
            </header>
            {visible
              .filter((row) => row.agent_type === kind.name)
              .map((row) => {
                const held =
                  claims.kind === 'ready'
                    ? claims.data.items.filter(
                        (item) =>
                          item.holder === row.id &&
                          item.attempt?.active &&
                          item.attempt.leaseUntilMs >
                            Date.parse(claims.data.generatedAt),
                      )
                    : [];
                return (
                  <article
                    key={row.id}
                    className="agents-instance"
                    data-selected={selected === row.id || undefined}
                    tabIndex={-1}
                    ref={(element) => {
                      if (element) rowRefs.current.set(row.id, element);
                      else rowRefs.current.delete(row.id);
                    }}
                    aria-label={`实例 ${row.id}`}
                  >
                    <div className="agents-instance__heading">
                      <code>{row.id}</code>
                      <Button
                        variant="danger"
                        size="sm"
                        disabled={
                          actions.busy || row.policy_revocation !== null
                        }
                        onClick={() =>
                          choose({ level: 'instance', target: row.id })
                        }
                      >
                        撤销此实例
                      </Button>
                    </div>
                    <dl>
                      <div>
                        <dt>状态</dt>
                        <dd>
                          {row.denial
                            ? DENIAL_LABEL[row.denial.reason]
                            : '授权可用（以调用时复核为准）'}
                          {row.denial?.by ? (
                            <>
                              {' '}
                              ·{' '}
                              <Button
                                variant="text"
                                size="sm"
                                onClick={() => openInstance(row.denial!.by!)}
                              >
                                查看祖先
                              </Button>
                            </>
                          ) : null}
                        </dd>
                      </div>
                      <div>
                        <dt>父实例</dt>
                        <dd>
                          {row.parent ? (
                            <Button
                              variant="text"
                              size="sm"
                              onClick={() => openInstance(row.parent!)}
                            >
                              {row.parent}
                            </Button>
                          ) : (
                            '—'
                          )}
                        </dd>
                      </div>
                      <div>
                        <dt>登记时间</dt>
                        <dd>{displayTime(row.registered_at)}</dd>
                      </div>
                      <div>
                        <dt>最近一次授权调用</dt>
                        <dd>
                          <time
                            dateTime={row.last_seen_at}
                            title={displayTime(row.last_seen_at)}
                          >
                            {relativeCall(row.last_seen_at, data.generated_at)}
                          </time>
                        </dd>
                      </div>
                      <div>
                        <dt>当前持有的认领</dt>
                        <dd>
                          {claims.kind === 'ready' ? (
                            <>
                              {claims.data.itemsTruncated ? '至少 ' : ''}
                              {held.length} 项
                              {held.map((item) => (
                                <Button
                                  key={item.id}
                                  variant="text"
                                  size="sm"
                                  onClick={() =>
                                    goTo({
                                      view: 'collab',
                                      project: item.project,
                                      panel: 'board',
                                      item: item.id,
                                    })
                                  }
                                >
                                  {item.title}
                                </Button>
                              ))}
                            </>
                          ) : claims.kind === 'absent' ? (
                            '—（未启用黑板）'
                          ) : (
                            '暂不可用'
                          )}
                        </dd>
                      </div>
                    </dl>
                    {row.policy_revocation ? (
                      <p className="policy-hint">
                        撤销理由：{row.policy_revocation.reason}
                        。实例不可恢复；请新建实例。
                      </p>
                    ) : null}
                  </article>
                );
              })}
            {!visible.some((row) => row.agent_type === kind.name) ? (
              <p className="policy-empty">当前筛选下没有实例。</p>
            ) : null}
          </section>
        ))}
      </Cell>
      {target ? (
        <Cell span={12} className="policy-cell">
          <section ref={formRef} tabIndex={-1} aria-label="撤销所选接入方">
            <CellHeader title="撤销所选接入方" />
            <RevokeForm
              key={`${target.level}:${target.target}`}
              initial={target}
              busy={actions.busy}
              previewId={previewId}
              preview={
                preview ? (
                  <ActionPreview
                    id={previewId}
                    content={preview.content}
                    disabled={actions.busy}
                    onConfirm={actions.confirmPreview}
                    onCancel={actions.closePreview}
                  />
                ) : null
              }
              onRequest={(request) => actions.previewRevoke('agents', request)}
              onEdit={() =>
                actions.closePreviewIf((open) => open.key === REVOKE_PREVIEW)
              }
            />
            <p className="policy-hint">
              实例撤销不能恢复；类型恢复须经过草案、校验和二次认证发布。
            </p>
          </section>
        </Cell>
      ) : null}
    </GridFrame>
  );
}
