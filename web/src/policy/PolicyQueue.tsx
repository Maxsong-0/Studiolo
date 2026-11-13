import { useEffect, useId, useMemo } from 'react';
import type { Ref } from 'react';
import { COMMAND_TARGETS } from '../commands/focusTarget';
import type { ListedDesired, PolicyStatus } from '../policyModel';
import { KIND_LABEL, STATUS_LABEL, shortId } from '../policyModel';
import { Button, Cell, CellHeader, GridFrame, StatusMark } from '../ui';
import { CapabilityDiff } from './CapabilityDiff';
import { compareCapability } from './capabilityDiff';
import { DraftPanel } from './DraftPanel';
import { displayTime, draftSubmitter } from './labels';
import { PolicyFeedback } from './PolicyFeedback';
import { Term } from './Term';
import type { PolicyActions } from './usePolicyActions';
import type { VersionBodies } from './useVersionBodies';
import '../styles/policy.css';
import '../styles/policy-tabs.css';

/**
 * 一份能力集草案改了哪几项能力（T-198）：排在校验报告前面的「现在／改后」表。
 * 对比基准是校验报告写的那一版（报告里的扩大项、收窄项就是相对它算的）；没有报告就按现在生效的那一版。
 * 基准正文读不到、对不上，只是这张表列不出来，说明原因；校验报告和发布照常。
 */
function DraftCapability({
  record,
  status,
  versions,
}: {
  record: ListedDesired;
  status: PolicyStatus;
  versions: VersionBodies;
}) {
  const report = record.report;
  const effective =
    status.entries.find(
      (entry) =>
        entry.kind === 'capability_set' && entry.subject === record.subject,
    )?.version ?? null;
  const base = report ? report.current_version : effective;
  const { bodies, read } = versions;
  useEffect(() => {
    if (base !== null) void read(base);
  }, [base, read]);

  const body = base === null ? null : (bodies.get(base) ?? null);
  const baseRecord = body?.kind === 'ready' ? body.record : null;
  // 读到的那一版必须真是这类 AI 的能力集；对不上就不比，不拿别的东西凑。
  const mismatched =
    baseRecord !== null &&
    (baseRecord.meta.kind !== 'capability_set' ||
      baseRecord.meta.subject !== record.subject);
  const before = baseRecord && !mismatched ? baseRecord.body : null;
  const comparison = useMemo(
    () => compareCapability(before, record.body),
    [before, record.body],
  );
  const waiting = base !== null && baseRecord === null;
  // 校验之后生效的版本又变了：这张表和报告比的仍是校验时的那一版，列头照实写，不叫它「生效中」。
  const stale = report !== null && effective !== report.current_version;
  const beforeLabel =
    before === null
      ? stale
        ? '校验时没有生效版本'
        : '没有生效版本'
      : stale
        ? `校验时的 v${base}`
        : `生效中 v${base}`;

  return (
    <div className="policy-capability" role="group" aria-label="能力差异">
      <h4 className="policy-capability__title">能力差异</h4>
      {record.body === null ? (
        <p className="policy-hint">
          这是一次撤销：发布后这类 AI
          回到零权限，下面「现在」一列的能力都会收回。
        </p>
      ) : base === null && !stale ? (
        <p className="policy-hint">
          现在没有生效的能力集：发布后，下面「改后」一列的每一项都是新给的。
        </p>
      ) : null}
      {stale ? (
        <p className="policy-hint">
          生效的版本已经不是校验时的那一版（现在
          {effective === null ? '没有生效版本' : `是 v${effective}`}
          ）：这张表和校验报告还是照
          {base === null ? '「没有生效版本」' : ` v${base} `}
          算的，重新校验后再看。
        </p>
      ) : null}
      {waiting && body?.kind === 'failed' ? (
        <p className="policy-hint">
          没读到 v{base} 的正文，这张表列不出来；校验报告和发布不受影响。{' '}
          <Button
            variant="text"
            size="sm"
            onClick={() => {
              if (base !== null) void read(base);
            }}
          >
            再读一次
          </Button>
        </p>
      ) : waiting ? (
        <p className="policy-hint" role="status">
          正在读 v{base} 的正文…
        </p>
      ) : mismatched ? (
        <p className="policy-hint">
          读到的 v{base} 不是这类 AI
          的能力集，这张表列不出来；看下面的校验报告。
        </p>
      ) : (
        <CapabilityDiff
          comparison={comparison}
          before={beforeLabel}
          after={record.body === null ? '撤销之后' : '这份草案'}
          verdicts={report}
        />
      )}
    </div>
  );
}

/**
 * 策略页「待处理」（T-198；设计规范第 9 节）：左边草案队列，右边选中那一份的细节——来历、能力差异
 * （只对能力集）、校验报告，以及发布、重新校验、撤回。没选时选第一份：只是把细节展开，不做任何动作。
 * 发布先展开执行预览，确认才发请求；动作结果落在右边这一格。
 */
export function PolicyQueue({
  status,
  desired,
  actionable,
  selected,
  actions,
  versions,
  queueRef,
  onPick,
}: {
  status: PolicyStatus;
  /** 全部草案（含已结束的）：发布预览要照它写明会取代哪几份、留下哪几份。 */
  desired: readonly ListedDesired[];
  /** 还没发布、还能处理的草案，按接口给的顺序。 */
  actionable: readonly ListedDesired[];
  selected: ListedDesired | null;
  actions: PolicyActions;
  versions: VersionBodies;
  /** 队列那一格：⌘K 和驾驶舱带路过来时把焦点放到它上面。 */
  queueRef: Ref<HTMLElement>;
  onPick: (record: ListedDesired) => void;
}) {
  const ids = useId();
  const { busy, notice, error } = actions;
  return (
    <GridFrame className="policy-grid">
      <Cell
        span={5}
        ref={queueRef}
        tabIndex={-1}
        data-command-target={COMMAND_TARGETS.policyQueue}
        className="policy-cell policy-queue-cell"
        aria-labelledby={`${ids}-queue`}
      >
        <CellHeader
          title={
            <span id={`${ids}-queue`}>
              <Term name="desired">待处理草案</Term>
            </span>
          }
          meta={`${actionable.length} 件`}
        />
        {actionable.length ? (
          <ul className="policy-queue" aria-label="待处理草案">
            {actionable.map((record) => {
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
                    aria-current={
                      selected?.id === record.id ? 'true' : undefined
                    }
                    aria-describedby={`${meta}-meta ${meta}-marks`}
                    disabled={busy}
                    onClick={() => onPick(record)}
                  >
                    {KIND_LABEL[record.kind]} {record.subject}
                  </button>
                  <span className="policy-queue__meta" id={`${meta}-meta`}>
                    草案 <code title={record.id}>{shortId(record.id)}</code>
                    {' · '}
                    {STATUS_LABEL[record.status]}，{draftSubmitter(record)}，
                    {displayTime(record.submitted_at)}
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
      </Cell>
      <Cell
        span={7}
        className="policy-cell policy-detail"
        aria-labelledby={`${ids}-detail`}
      >
        <CellHeader title={<span id={`${ids}-detail`}>选中的草案</span>} />
        <PolicyFeedback origin="queue" notice={notice} error={error} />
        {/* 在「提交」里交成功后人会被带到这里：那一句结果跟着过来；没交成的原因留在提交那一格。 */}
        <PolicyFeedback origin="submit" notice={notice} error={null} />
        {selected ? (
          <DraftPanel
            key={selected.id}
            record={selected}
            title={`${KIND_LABEL[selected.kind]} ${selected.subject}`}
            label={`${KIND_LABEL[selected.kind]} ${selected.subject}`}
            origin="queue"
            status={status}
            actions={actions}
            drafts={desired}
          >
            {selected.kind === 'capability_set' ? (
              <DraftCapability
                record={selected}
                status={status}
                versions={versions}
              />
            ) : null}
          </DraftPanel>
        ) : (
          <p className="policy-hint">
            队列是空的。自己写的草案在「提交」里交；AI 提的会出现在这里。
          </p>
        )}
      </Cell>
    </GridFrame>
  );
}
