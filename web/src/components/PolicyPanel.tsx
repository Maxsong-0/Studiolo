import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { COMMAND_TARGETS } from '../commands/focusTarget';
import {
  activateFailure,
  activatePolicy,
  needsReauthentication,
  readPolicySnapshot,
  readVersion,
  revokeAgent,
  submitDesired,
  validateDesired,
  withdrawDesired,
} from '../policyApi';
import type {
  ActivateRequest,
  DesiredInput,
  DesiredRecord,
  ListedDesired,
  PolicyStatus,
  RevokeRequest,
  VersionMeta,
  VersionRecord,
} from '../policyModel';
import {
  KIND_LABEL,
  LEVEL_LABEL,
  STATUS_LABEL,
  isOpenStatus,
  shortHash,
  remainingAfter,
  shortId,
  supersededBy,
} from '../policyModel';
import { ActionPreview } from '../policy/ActionPreview';
import {
  newerDraftReason,
  publishHint,
  publishPreview,
  publishedNotice,
  releasePreview,
  revokePreview,
  rollbackPreview,
  type PreviewContent,
} from '../policy/previews';
import { Term } from '../policy/Term';
import { POLICY_TERMS } from '../policy/terms';
import { Button, Cell, CellHeader, GridFrame, StatusMark } from '../ui';
import { cx } from '../ui/dom';
import { useViewLocation } from '../useViewLocation';
import { DesiredForm, RevokeForm } from './PolicyForms';
import { ChangeList, PolicyReport } from './PolicyReport';
import { PolicyStepUp } from './PolicyStepUp';
import '../styles/policy.css';

type Loaded = {
  status: PolicyStatus;
  desired: ListedDesired[];
  versions: VersionMeta[];
};
/** 反馈落在发起动作的那一格里：人在哪格点的，就在哪格看结果。 */
type Origin = 'effective' | 'submit' | 'queue' | 'versions' | 'revoke';
/**
 * 一次 L3 动作：请求本体、成功提示里的名字、给人看的动作说明、发起的格子，以及执行预览里列出的
 * 同一对象其余未发布草案份数：会被一并标为已取代的较早草案、照旧待处理的更新草案（只有发布草案
 * 才可能不是 0）。
 */
type Activation = {
  request: ActivateRequest;
  label: string;
  action: string;
  origin: Origin;
  supersedes: number;
  remains: number;
};
/** 被二次认证挡下、等人就地认证后原样重试的那一个动作；`attempt` 只用于措辞。 */
type Pending = Activation & { attempt: number };
/** 收场时服务端状态没变，不必刷新列表。 */
type Unchanged = { notice: string; reload: false };
type Feedback = { origin: Origin; text: string };
/**
 * 打开着的执行预览：同一时刻只有一个。`key` 标明它属于哪个按钮
 * （`publish:<草案 id>`、`rollback:<版本号>`、`release`、`revoke`）；请求在打开时就定下，确认时原样发出。
 */
type Preview = { key: string; content: PreviewContent } & (
  | { kind: 'activate'; activation: Activation }
  | { kind: 'revoke'; request: RevokeRequest }
);

/**
 * 位置 `item=desired` 表示「滚到并聚焦待处理草案队列」（⌘K 的「打开策略待处理队列」「发布策略」写的就是它，
 * 见 commands/registry.ts）；其余 item 仍是版本号（BUG-015/T-098）。
 */
const QUEUE_ITEM = 'desired';

function when(value: string) {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

function CellFeedback({
  origin,
  notice,
  error,
}: {
  origin: Origin;
  notice: Feedback | null;
  error: Feedback | null;
}) {
  return (
    <>
      {notice?.origin === origin ? (
        <p className="policy-feedback" role="status">
          {notice.text}
        </p>
      ) : null}
      {error?.origin === origin ? (
        <p className="policy-feedback policy-feedback--error" role="alert">
          {error.text}
        </p>
      ) : null}
    </>
  );
}

/**
 * 策略平面视图（T-130 设计规范第 3、9 节）：12 列网格——
 * 第一行生效中各项与生效中的撤销接入；第二行提交规则草案 ｜ 待处理草案（选中草案的校验报告与发布）；
 * 第三行版本与回滚 ｜ 撤销接入。全部动作先校验后动手，永不自动发布；
 * 发布、回滚、解除全局撤销与三级撤销接入先在按钮上方展开「执行预览」，确认后才发请求。
 */
export function PolicyPanel({ initial }: { initial: Loaded }) {
  const [data, setData] = useState<Loaded>(initial);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<Feedback | null>(null);
  const [error, setError] = useState<Feedback | null>(null);
  // 选中只记草案号，记录每次从刚读到的列表里取：重读后状态、报告和先后都是新的，不会停在旧对象上。
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [version, setVersion] = useState<VersionRecord | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const ids = useId();
  // 同一时刻只有一个执行预览，一个 id 就够：触发按钮展开时用 aria-controls 指向它。
  const previewId = `${ids}-preview`;
  const queueRef = useRef<HTMLElement>(null);
  // 在看哪一版的正文属于「位置」：只记版本号，重挂/刷新/分享链接后回读；
  // 版本正文与规则草案正文绝不进 URL。
  const { location, select } = useViewLocation();
  const restored = useRef<string | null>(null);
  // 控制器必须在挂载时重建：StrictMode 的装载—卸载—再装载会先 abort 一次，沿用同一个
  // 已中止的控制器会让此后每个动作在发请求前就被取消，界面表现为点了毫无反应（BUG-016，
  // 与其余四处 useRef(new AbortController()) 的写法对齐）。
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    lifetime.current = new AbortController();
    return () => lifetime.current.abort();
  }, []);

  // 位置里带着版本号就把那一份正文读回来（会话重新核对导致整页重挂、刷新、
  // 或别人打开同一个链接）。读失败就停在列表：不弹通知、不动 URL、不猜别的版本。
  useEffect(() => {
    const wanted = location.item;
    if (!wanted || wanted === QUEUE_ITEM || restored.current === wanted) return;
    const meta = data.versions.find((row) => String(row.version) === wanted);
    if (!meta) return;
    restored.current = wanted;
    const signal = lifetime.current.signal;
    readVersion(meta.version, signal)
      .then((record) => {
        if (!signal.aborted) setVersion(record);
      })
      .catch(() => undefined);
  }, [location.item, data.versions]);

  // `item=desired`：把待处理草案队列滚进视野并聚焦（不改写位置、不报错）。
  // 只在位置变成它的那一刻做一次，刷新列表不会把焦点再拽回来；别的对话框开着就只滚不抢焦点。
  useEffect(() => {
    if (location.item !== QUEUE_ITEM) return;
    const queue = queueRef.current;
    if (!queue) return;
    queue.scrollIntoView({ block: 'start' });
    if (!document.querySelector('dialog[open]'))
      queue.focus({ preventScroll: true });
  }, [location.item]);

  const reload = useCallback(async () => {
    const signal = lifetime.current.signal;
    const snapshot = await readPolicySnapshot(signal);
    if (!signal.aborted) setData(snapshot);
  }, []);

  // 动作的返回值＝要显示的一句话；`reload: false` 用于服务端明确没有改动状态的收场
  // （被 L3 挡下），那时再拉三个列表只会让 busy 多挂一会儿，把人认证完的重试吞掉。
  // 任何动作开始都收起打开着的执行预览：预览只描述发起那一刻的状态。
  async function act(
    origin: Origin,
    operation: () => Promise<string | Unchanged>,
  ) {
    if (busy) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    setPreview(null);
    try {
      const result = await operation();
      if (lifetime.current.signal.aborted) return;
      setNotice({
        origin,
        text: typeof result === 'string' ? result : result.notice,
      });
      if (typeof result === 'string') await reload();
    } catch (cause) {
      if (lifetime.current.signal.aborted) return;
      setError({
        origin,
        text:
          cause instanceof Error ? cause.message : '操作未完成，请重新核对。',
      });
      await reload().catch(() => undefined);
    } finally {
      if (!lifetime.current.signal.aborted) setBusy(false);
    }
  }

  function submit(input: DesiredInput) {
    void act('submit', async () => {
      const record = await submitDesired(input, lifetime.current.signal);
      setSelectedId(record.id);
      return record.report?.ok
        ? `已提交并校验通过（${record.id}）；发布前请逐条确认扩大项。`
        : `已提交但校验未通过（${record.id}）；请按逐条问题修改后重新提交。`;
    });
  }
  function validate(record: DesiredRecord) {
    void act('queue', async () => {
      const updated = await validateDesired(record.id, lifetime.current.signal);
      setSelectedId(updated.id);
      return `已重新校验：${STATUS_LABEL[updated.status]}。`;
    });
  }
  function withdraw(record: DesiredRecord) {
    void act('queue', async () => {
      await withdrawDesired(record.id, lifetime.current.signal);
      setSelectedId(null);
      return '已撤回。';
    });
  }
  // 认证后重试沿用同一个 request：人审过哪一份生效中内容就按哪一份发布，
  // 期间生效中内容变了由服务端以 hash_mismatch 挡下，绝不悄悄改用新的。
  function activate(activation: Activation, attempt = 0) {
    const { request, label, origin, supersedes, remains } = activation;
    void act(origin, async () => {
      try {
        const outcome = await activatePolicy(request, lifetime.current.signal);
        setSelectedId(null);
        setVersion(null);
        // 版本列表已经变了，位置里那个版本号不再是「刚才在看的那一份」。
        restored.current = null;
        select({ item: null });
        if (outcome.kind === 'released')
          return `已解除全局撤销${outcome.audit_confirmed ? '' : '（审计未确认，请核对账本）'}。`;
        return `${label}：已发布为 v${outcome.activation.version}，生效中摘要 ${shortHash(
          outcome.activation.effective_hash,
        )}${outcome.activation.cleared_type_revocation ? '，并解除了该类型的撤销接入' : ''}${publishedNotice(
          supersedes,
          remains,
        )}${outcome.audit_confirmed ? '' : '（审计未确认，请核对账本）'}。`;
      } catch (cause) {
        // 403「需要二次认证」不是终点：发布没有发生，只差一次一次性验证码。
        if (needsReauthentication(cause)) {
          setPending({ ...activation, attempt });
          return {
            notice: `${activation.action} 需要二次认证：请在弹出的窗口输入一次性验证码，通过后会自动继续这一次动作。`,
            reload: false,
          };
        }
        throw new Error(activateFailure(cause));
      }
    });
  }
  function retryAfterStepUp() {
    const next = pending;
    setPending(null);
    if (next) activate(next, next.attempt + 1);
  }
  function revoke(request: RevokeRequest) {
    void act('revoke', async () => {
      const outcome = await revokeAgent(request, lifetime.current.signal);
      return outcome.created
        ? `已撤销接入：${LEVEL_LABEL[outcome.level]} ${outcome.target}（即刻生效）。`
        : `${LEVEL_LABEL[outcome.level]} ${outcome.target} 已处于撤销接入中。`;
    });
  }
  function open(meta: VersionMeta) {
    void act('versions', async () => {
      setVersion(await readVersion(meta.version, lifetime.current.signal));
      // 先记下再写位置：回读 effect 据此认出这一份已经在手，不重复请求。
      restored.current = String(meta.version);
      select({ item: restored.current });
      return `已读取 v${meta.version}。`;
    });
  }

  // ---------- 执行预览：打开不发请求，确认才走上面的原流程 ----------

  /** 触发按钮再点一次就收起（它带 aria-expanded）。 */
  function toggleActivation(
    key: string,
    activation: Activation,
    content: PreviewContent,
  ) {
    setPreview((current) =>
      current?.key === key
        ? null
        : { key, content, kind: 'activate', activation },
    );
  }
  function previewRevoke(request: RevokeRequest) {
    setPreview({
      key: 'revoke',
      content: revokePreview(request),
      kind: 'revoke',
      request,
    });
  }
  function confirmPreview() {
    const current = preview;
    if (!current) return;
    setPreview(null);
    if (current.kind === 'activate') activate(current.activation);
    else revoke(current.request);
  }
  function previewFor(key: string) {
    if (preview?.key !== key) return null;
    return (
      <ActionPreview
        id={previewId}
        content={preview.content}
        disabled={busy}
        onConfirm={confirmPreview}
        onCancel={() => setPreview(null)}
      />
    );
  }
  /** 触发按钮与它的预览框的关联：展开时指向框。 */
  function trigger(key: string) {
    const expanded = preview?.key === key;
    return {
      'aria-expanded': expanded,
      'aria-controls': expanded ? previewId : undefined,
    };
  }
  function pick(record: DesiredRecord) {
    setSelectedId(record.id);
    // 换了草案，发布预览描述的已不是眼前这一份。
    setPreview((current) =>
      current?.key.startsWith('publish:') ? null : current,
    );
  }

  const { status, desired, versions } = data;
  const actionable = desired.filter((record) => isOpenStatus(record.status));
  const selected = desired.find((record) => record.id === selectedId) ?? null;
  // 发布选中这份时：比它早的一并标为已取代，比它更新的（别的提交者交的）照旧待处理；
  // 同一提交者还有更新的就发不出，只说为什么。
  const superseded = selected ? supersededBy(selected, desired) : [];
  const remaining = selected ? remainingAfter(selected, desired) : [];
  const newerReason = selected ? newerDraftReason(selected.blocked_by) : null;
  const selectedHint =
    newerReason ?? publishHint(superseded.length, remaining.length);
  const heading = {
    effective: `${ids}-effective`,
    submit: `${ids}-submit`,
    queue: `${ids}-queue`,
    versions: `${ids}-versions`,
    revoke: `${ids}-revoke`,
  };
  const feedback = { notice, error };

  return (
    <div className="policy">
      <GridFrame className="policy-grid">
        {/* ---------- 第一行：生效中 ---------- */}
        <Cell
          span={12}
          className="policy-cell policy-effective"
          aria-labelledby={heading.effective}
        >
          <CellHeader
            title={
              <span id={heading.effective}>
                <Term name="effective" />
              </span>
            }
            meta={
              <>
                {POLICY_TERMS.sha256.human}{' '}
                <code className="policy-hash" title={status.effective_hash}>
                  {shortHash(status.effective_hash)}
                </code>
              </>
            }
          />
          <CellFeedback origin="effective" {...feedback} />
          <p className="policy-note">
            待处理草案 {status.pending_desired} 件；
            {status.l3_available
              ? '本入口可以经一次性验证码发布。'
              : '本入口没有一次性验证码路径，只能查看与撤销接入。'}
          </p>
          {status.global_revoked ? (
            <div className="policy-global">
              <p className="policy-global__alert" role="alert">
                全局撤销生效中：所有 AI 都被拒绝。解除需要二次认证。
              </p>
              {previewFor('release')}
              <div className="policy-actions">
                <Button
                  variant="danger"
                  disabled={busy || !status.l3_available}
                  {...trigger('release')}
                  onClick={() =>
                    toggleActivation(
                      'release',
                      {
                        request: {
                          release_global_revocation: true,
                          expected_effective_hash: status.effective_hash,
                        },
                        label: '解除全局撤销',
                        action: '解除全局撤销',
                        origin: 'effective',
                        supersedes: 0,
                        remains: 0,
                      },
                      releasePreview(status.effective_hash),
                    )
                  }
                >
                  解除全局撤销（需二次认证）
                </Button>
              </div>
            </div>
          ) : null}
          <div className="policy-effective__body">
            <table className="policy-table" aria-label="生效中的各项">
              <thead>
                <tr>
                  <th scope="col">类别</th>
                  <th scope="col">
                    <Term name="subject" />
                  </th>
                  <th scope="col" className="is-number">
                    版本
                  </th>
                  <th scope="col" className="is-number">
                    <Term name="sha256" />
                  </th>
                  <th scope="col" className="is-number">
                    发布时间
                  </th>
                </tr>
              </thead>
              <tbody>
                {status.entries.length ? (
                  status.entries.map((entry) => (
                    <tr key={`${entry.kind}:${entry.subject}`}>
                      <td className="policy-table__kind">
                        {KIND_LABEL[entry.kind]}
                      </td>
                      <td className="policy-table__subject">{entry.subject}</td>
                      <td className="is-number">v{entry.version}</td>
                      <td className="is-number">
                        <code title={entry.body_sha256}>
                          {shortHash(entry.body_sha256)}
                        </code>
                      </td>
                      <td className="is-number">{when(entry.activated_at)}</td>
                    </tr>
                  ))
                ) : (
                  <tr>
                    <td colSpan={5} className="policy-table__empty">
                      尚无任何生效母本：所有 AI
                      类型都是零权限，直到发布其能力集。
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
            <div className="policy-revocations">
              <h3 className="policy-subhead">
                生效中的撤销接入
                <span className="policy-subhead__count">
                  {status.revocations.length}
                </span>
              </h3>
              {status.revocations.length ? (
                <ul aria-label="生效中的撤销接入">
                  {status.revocations.map((item) => (
                    <li
                      key={item.seq}
                      className={cx(item.level === 'global' && 'is-global')}
                    >
                      <span className="policy-revocations__level">
                        {item.level === 'global' ? (
                          <span className="ui-signal-mark" aria-hidden="true" />
                        ) : null}
                        {LEVEL_LABEL[item.level]}
                      </span>
                      <span className="policy-revocations__what">
                        <code>{item.target}</code>
                        <span className="policy-revocations__reason">
                          {item.reason}
                        </span>
                      </span>
                      <span className="policy-revocations__when">
                        {when(item.revoked_at)}
                      </span>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="policy-hint">没有生效中的撤销接入。</p>
              )}
            </div>
          </div>
        </Cell>

        {/* ---------- 第二行：提交规则草案 ｜ 待处理草案 ---------- */}
        <Cell
          span={6}
          className="policy-cell policy-submit"
          aria-labelledby={heading.submit}
        >
          <CellHeader
            title={
              <span id={heading.submit}>
                <Term name="desired">提交规则草案</Term>
              </span>
            }
          />
          <CellFeedback origin="submit" {...feedback} />
          <DesiredForm busy={busy} onSubmit={submit} />
        </Cell>
        <Cell
          span={6}
          ref={queueRef}
          tabIndex={-1}
          data-command-target={COMMAND_TARGETS.policyQueue}
          className="policy-cell policy-queue-cell"
          aria-labelledby={heading.queue}
        >
          <CellHeader
            title={
              <span id={heading.queue}>
                <Term name="desired">待处理草案</Term>
              </span>
            }
            meta={`${actionable.length} 件`}
          />
          <CellFeedback origin="queue" {...feedback} />
          {actionable.length ? (
            <ul className="policy-queue" aria-label="待处理草案">
              {actionable.map((record) => {
                const current = selected?.id === record.id;
                const expansions = record.report?.expansions.length ?? null;
                // 同一对象有几份未发布草案时标出哪份最新（服务端判定）；较早的那份只有同一提交者
                // 还有更新的才不能发布（T-181，用户 2026-10-16 定）。
                const order = record.newer_open.length
                  ? record.blocked_by.length
                    ? 'blocked'
                    : 'older'
                  : actionable.some((other) =>
                        other.newer_open.includes(record.id),
                      )
                    ? 'newest'
                    : null;
                const meta = `${ids}-queue-${record.id}`;
                return (
                  <li key={record.id}>
                    <button
                      type="button"
                      className="policy-queue__pick"
                      aria-current={current ? 'true' : undefined}
                      aria-describedby={`${meta}-meta ${meta}-marks`}
                      disabled={busy}
                      onClick={() => pick(record)}
                    >
                      {KIND_LABEL[record.kind]} {record.subject}
                    </button>
                    <span className="policy-queue__meta" id={`${meta}-meta`}>
                      草案 <code title={record.id}>{shortId(record.id)}</code>
                      {' · '}
                      {STATUS_LABEL[record.status]}
                      {record.submitter_kind === 'agent'
                        ? `，由 ${record.agent_type ?? '未知类型'} 实例 ${record.agent_instance ?? '未知'} 提交`
                        : '，主人提交'}
                      ，{when(record.submitted_at)}
                    </span>
                    <span className="policy-queue__marks" id={`${meta}-marks`}>
                      {order === 'newest' ? (
                        <StatusMark state="active">最新</StatusMark>
                      ) : order === 'blocked' ? (
                        <StatusMark state="dormant">较早，不能发布</StatusMark>
                      ) : order === 'older' ? (
                        <StatusMark state="dormant">较早</StatusMark>
                      ) : null}
                      {/* 接口没给报告就照实说没有，不猜它扩大了几项 */}
                      {expansions === null ? (
                        <StatusMark state="dormant">无校验报告</StatusMark>
                      ) : expansions > 0 ? (
                        <StatusMark
                          className="policy-queue__expansions"
                          state="alert"
                        >
                          扩大 {expansions}
                        </StatusMark>
                      ) : (
                        <StatusMark state="pending">无扩大项</StatusMark>
                      )}
                    </span>
                  </li>
                );
              })}
            </ul>
          ) : (
            <p className="policy-hint">没有待处理的规则草案。</p>
          )}
          {selected?.report ? (
            <div
              className="policy-selected"
              role="group"
              aria-labelledby={`${ids}-selected`}
            >
              <div className="policy-selected__head">
                <h3 className="policy-selected__title" id={`${ids}-selected`}>
                  {KIND_LABEL[selected.kind]} {selected.subject}
                </h3>
                <p className="policy-selected__meta">
                  <code>{selected.id}</code> · {STATUS_LABEL[selected.status]}
                </p>
              </div>
              <PolicyReport report={selected.report} />
              {isOpenStatus(selected.status) ? (
                <>
                  {selectedHint ? (
                    <p className="policy-hint">{selectedHint}</p>
                  ) : null}
                  {previewFor(`publish:${selected.id}`)}
                  <div className="policy-actions">
                    <Button
                      variant="danger"
                      disabled={
                        busy ||
                        !selected.report.ok ||
                        !status.l3_available ||
                        newerReason !== null
                      }
                      {...trigger(`publish:${selected.id}`)}
                      onClick={() =>
                        selected.report &&
                        toggleActivation(
                          `publish:${selected.id}`,
                          {
                            request: {
                              desired_id: selected.id,
                              expected_effective_hash: status.effective_hash,
                            },
                            label: `${KIND_LABEL[selected.kind]} ${selected.subject}`,
                            action: `发布 ${KIND_LABEL[selected.kind]} ${selected.subject}`,
                            origin: 'queue',
                            supersedes: superseded.length,
                            remains: remaining.length,
                          },
                          publishPreview(
                            selected,
                            selected.report,
                            status,
                            superseded,
                            remaining,
                          ),
                        )
                      }
                    >
                      发布（需二次认证）
                    </Button>
                    <Button
                      size="sm"
                      disabled={busy}
                      onClick={() => validate(selected)}
                    >
                      重新校验
                    </Button>
                    <Button
                      variant="text"
                      size="sm"
                      disabled={busy}
                      onClick={() => withdraw(selected)}
                    >
                      撤回
                    </Button>
                  </div>
                </>
              ) : null}
            </div>
          ) : null}
        </Cell>

        {/* ---------- 第三行：版本与回滚 ｜ 撤销接入 ---------- */}
        <Cell
          span={7}
          tabIndex={-1}
          data-command-target={COMMAND_TARGETS.policyVersions}
          className="policy-cell policy-versions"
          aria-labelledby={heading.versions}
        >
          <CellHeader
            title={
              <span id={heading.versions}>
                版本与<Term name="rollback">回滚</Term>
              </span>
            }
            meta={`${versions.length} 版`}
          />
          <CellFeedback origin="versions" {...feedback} />
          {versions.length ? (
            <ul className="policy-version-list" aria-label="版本列表">
              {versions.map((meta) => {
                const key = `rollback:${meta.version}`;
                const reading = version?.meta.version === meta.version;
                return (
                  <li key={meta.version} className="policy-version">
                    <button
                      type="button"
                      className="policy-version__open"
                      aria-current={reading ? 'true' : undefined}
                      disabled={busy}
                      onClick={() => open(meta)}
                    >
                      v{meta.version}
                    </button>
                    <span className="policy-version__what">
                      <span className="policy-version__subject">
                        {KIND_LABEL[meta.kind]} {meta.subject}
                        {meta.retired ? '（撤销）' : ''}
                      </span>
                      {/* 发布时的扩大项是已经确认过的历史，不等人也不是故障：写成中性色文字，不标红 */}
                      <span className="policy-version__meta">
                        {meta.rollback_of !== null
                          ? `回滚自 v${meta.rollback_of} · `
                          : ''}
                        {meta.activated_by} · {when(meta.activated_at)}
                        {meta.expansions.length ? (
                          <>
                            {' · '}
                            <span className="policy-nowrap">
                              发布时扩大 {meta.expansions.length} 项
                            </span>
                          </>
                        ) : null}
                      </span>
                    </span>
                    {previewFor(key)}
                    <Button
                      variant="danger"
                      size="sm"
                      className="policy-version__rollback"
                      disabled={busy || !status.l3_available}
                      {...trigger(key)}
                      onClick={() =>
                        toggleActivation(
                          key,
                          {
                            request: {
                              rollback_to: meta.version,
                              expected_effective_hash: status.effective_hash,
                            },
                            label: `回滚到 v${meta.version}`,
                            action: `回滚到 v${meta.version}`,
                            origin: 'versions',
                            supersedes: 0,
                            remains: 0,
                          },
                          rollbackPreview(meta, status),
                        )
                      }
                    >
                      回滚到此版本（需二次认证）
                    </Button>
                  </li>
                );
              })}
            </ul>
          ) : (
            <p className="policy-hint">尚无版本。</p>
          )}
          {version ? (
            <div className="policy-version-body">
              <div className="policy-version-body__head">
                <h3 className="policy-subhead">
                  v{version.meta.version} {KIND_LABEL[version.meta.kind]}{' '}
                  {version.meta.subject}
                </h3>
                <p className="policy-version-body__hash">
                  {POLICY_TERMS.sha256.human}{' '}
                  <code title={version.meta.body_sha256}>
                    {shortHash(version.meta.body_sha256)}
                  </code>
                </p>
              </div>
              <pre>{version.body ?? '（撤销版本，无正文）'}</pre>
              <p className="policy-version-body__label">
                发布时记录的扩大项（相对上一份生效中）
              </p>
              {version.meta.expansions.length ? (
                <ChangeList
                  items={version.meta.expansions}
                  kind="expansion"
                  historical
                  label={`v${version.meta.version} 发布时的扩大项`}
                />
              ) : (
                <p className="policy-hint">这一版发布时没有记录扩大项。</p>
              )}
              <p className="policy-hint">
                版本记录只保存发布时的扩大项；逐项差异只在规则草案的校验报告里提供。
              </p>
            </div>
          ) : null}
        </Cell>
        <Cell
          span={5}
          className="policy-cell policy-revoke"
          aria-labelledby={heading.revoke}
        >
          <CellHeader
            title={
              <span id={heading.revoke}>
                <Term name="revoke">撤销接入（实例/类型/全局）</Term>
              </span>
            }
          />
          <CellFeedback origin="revoke" {...feedback} />
          <RevokeForm
            busy={busy}
            preview={previewFor('revoke')}
            previewId={previewId}
            onRequest={previewRevoke}
            onEdit={() =>
              setPreview((current) =>
                current?.kind === 'revoke' ? null : current,
              )
            }
          />
        </Cell>
      </GridFrame>

      {pending ? (
        <PolicyStepUp
          action={pending.action}
          attempt={pending.attempt}
          onAuthenticated={retryAfterStepUp}
          onClose={() => setPending(null)}
        />
      ) : null}
    </div>
  );
}
