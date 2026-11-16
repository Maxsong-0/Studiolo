import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { ActionPreview } from '../policy/ActionPreview';
import { PolicyFeedback } from '../policy/PolicyFeedback';
import {
  publishPreview,
  withdrawPreview,
  withdrawVerb,
} from '../policy/previews';
import { stepUpNote } from '../policy/recentAuth';
import type { PolicyActions } from '../policy/usePolicyActions';
import type { RecentAuthRead } from '../policy/useRecentAuth';
import { readPolicyStatus } from '../policyApi';
import type { ListedDesired } from '../policyModel';
import { remainingAfter, supersededBy } from '../policyModel';
import { Button } from '../ui';
import {
  VERDICT_TEXT,
  approveBlocker,
  publishKey,
  reviewLabel,
  reviewOrigin,
  reviewSignature,
  reviewVerdict,
  validatedNotice,
  withdrawKey,
  withdrawnNotice,
  type ReviewVerdict,
} from './review';
import '../styles/cockpit.css';

/** 驾驶舱交给队列的就地审批：全页那一份写动作，和近期二次认证的那一份读取。 */
export interface QueueReview {
  actions: PolicyActions;
  auth: RecentAuthRead;
}

/** 按了「批准并发布」之后、展开预览之前的那一步核对走到哪了。 */
type Check =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | Exclude<ReviewVerdict, { kind: 'ready' }>
  | { kind: 'failed'; message: string };

const IDLE: Check = { kind: 'idle' };

/** 焦点已经没落在任何东西上（按钮被撤掉之后浏览器把它放回 body）。 */
function focusLost(): boolean {
  const active = document.activeElement;
  return active === null || active === document.body;
}

/**
 * 选中的那一行是策略草案时，就地细节下面的「批准并发布」「退回」（T-200；设计规范第 6、17 节）。
 *
 * - 批准：先核对两样（各读一次）——现在生效的内容、二次认证状态。草案的校验报告是照现在生效的内容算的，
 *   才展开执行预览（文字与策略页同一个函数）；确认才发请求，请求带的是报告的基准摘要。
 *   报告旧了（校验之后生效内容变过）就不让批，给「重新校验」。
 * - 退回（自己交的草案叫撤回）：先展开确认框，确认才撤回。
 * - 这两个按钮也是 A／R 两个键按到的东西：键只是替人点一下按钮，所以键和鼠标走的是同一条路，
 *   而且都只到「打开确认」为止，不直接写。
 *
 * 写动作、二次认证的重试、做完重读都在 policy/usePolicyActions 里；这里不持有凭据、不记认证结果。
 */
export function InlineReview({
  desired,
  drafts,
  actions,
  auth,
  onActed,
}: {
  desired: ListedDesired;
  /** 整份草案列表：执行预览要照它写明会取代哪几份、留下哪几份。 */
  drafts: readonly ListedDesired[];
  actions: PolicyActions;
  auth: RecentAuthRead;
  /**
   * 一件写动作交出去了（确认了发布或退回，或者点了校验）；参数是对象的名字。队列据此记下是哪一行：
   * 出错时点明是哪一份，那一行不见之后把焦点交给顶上来的那一行。
   */
  onActed: (label: string) => void;
}) {
  const ids = useId();
  const previewId = `${ids}-preview`;
  const container = useRef<HTMLDivElement>(null);
  const origin = reviewOrigin(desired.id);
  const approveKey = publishKey(desired.id);
  const rejectKey = withdrawKey(desired.id);
  const label = reviewLabel(desired);
  const verb = withdrawVerb(desired);
  const blocker = approveBlocker(desired);
  const signature = reviewSignature(desired);
  const open =
    actions.preview?.key === approveKey || actions.preview?.key === rejectKey
      ? actions.preview
      : null;

  // 核对的结论只对当时那一份草案算数：草案变了（重新校验过），结论跟着作废。
  const [checked, setChecked] = useState({ signature, check: IDLE });
  const check = checked.signature === signature ? checked.check : IDLE;
  // 在途的那一次核对。
  const checking = useRef<AbortController | null>(null);
  // 预览是照哪一份草案开的。
  const opened = useRef<string | null>(null);
  // 卸载、草案变了的时候要收预览：用最新的那一份 actions。
  const latest = useRef(actions);
  useLayoutEffect(() => {
    latest.current = actions;
  });

  // 人走开了（换了一行、这一行放到了稍后、离开驾驶舱）：在途的核对作废，预览收起——
  // 预览只描述眼前这一份。
  useEffect(
    () => () => {
      checking.current?.abort();
      latest.current.closePreviewIf(
        (preview) => preview.key === approveKey || preview.key === rejectKey,
      );
    },
    [approveKey, rejectKey],
  );

  // 预览开着的时候草案变了（别处重新校验过、状态变了）：收起，让人照新的再看一遍。
  const previewOpen = open !== null;
  useEffect(() => {
    if (previewOpen && opened.current !== signature)
      latest.current.closePreview();
  }, [previewOpen, signature]);

  // 键盘接着走：报告旧了，焦点交给「重新校验」；点了校验、校验完那个按钮撤掉了，焦点交给「批准并发布」。
  // 后一步等动作做完再交（做的时候按钮是禁用的，接不住焦点），而且只在焦点没落在别处时才动。
  const stale = check.kind === 'stale';
  const { busy } = actions;
  // 刚点过校验，还没把焦点交出去。
  const revalidating = useRef(false);
  useEffect(() => {
    const within = (selector: string) =>
      container.current?.querySelector<HTMLElement>(selector);
    if (stale) {
      within('[data-review-validate]')?.focus();
      return;
    }
    if (!revalidating.current || busy) return;
    revalidating.current = false;
    if (focusLost()) within('[data-queue-approve]')?.focus();
  }, [stale, busy]);

  function approve() {
    const report = desired.report;
    if (
      !report ||
      blocker !== null ||
      actions.busy ||
      actions.pending !== null ||
      check.kind === 'checking'
    )
      return;
    // 再按一次：收起。
    if (actions.preview?.key === approveKey) {
      actions.closePreview();
      return;
    }
    actions.closePreview();
    checking.current?.abort();
    const controller = new AbortController();
    checking.current = controller;
    const { signal } = controller;
    setChecked({ signature, check: { kind: 'checking' } });
    void (async () => {
      try {
        // 一个接一个读，不并发：因子状态每读一次就作废上一份在途的读取。
        const status = await readPolicyStatus(signal);
        const recent = await auth.read(signal);
        if (signal.aborted) return;
        const verdict = reviewVerdict(desired, status, recent);
        if (verdict.kind !== 'ready') {
          setChecked({ signature, check: verdict });
          return;
        }
        setChecked({ signature, check: IDLE });
        const superseded = supersededBy(desired, drafts);
        const remaining = remainingAfter(desired, drafts);
        opened.current = signature;
        actions.togglePreview(
          approveKey,
          {
            request: {
              desired_id: desired.id,
              // 报告是照哪一份生效内容算的，就按哪一份提交（核对过它就是现在这一份）。
              expected_effective_hash: report.base_effective_hash,
            },
            label,
            action: `发布 ${label}`,
            origin,
            supersedes: superseded.length,
            remains: remaining.length,
          },
          publishPreview(desired, report, status, superseded, remaining, {
            expansionsWhere: '上面已逐条列出',
            stepUp: stepUpNote(recent, auth.now()),
          }),
        );
      } catch (cause) {
        if (signal.aborted) return;
        setChecked({
          signature,
          check: {
            kind: 'failed',
            message:
              cause instanceof Error ? cause.message : '没读到现在生效的内容。',
          },
        });
      }
    })();
  }

  function reject() {
    if (actions.busy || actions.pending !== null) return;
    checking.current?.abort();
    setChecked({ signature, check: IDLE });
    opened.current = signature;
    actions.previewWithdraw(
      rejectKey,
      origin,
      desired,
      withdrawPreview(desired),
      withdrawnNotice(desired),
    );
  }

  function confirm() {
    onActed(label);
    actions.confirmPreview();
  }

  function validate() {
    if (actions.busy || actions.pending !== null) return;
    checking.current?.abort();
    revalidating.current = true;
    onActed(label);
    actions.validate(origin, desired, (updated) =>
      validatedNotice(desired, updated.status),
    );
  }

  const approveOpen = open?.key === approveKey;
  const rejectOpen = open?.key === rejectKey;
  return (
    <div ref={container} className="cockpit-review">
      <PolicyFeedback
        origin={origin}
        notice={actions.notice}
        error={actions.error}
      />
      {blocker !== null ? (
        <p className="cockpit-review__hint">{blocker}</p>
      ) : null}
      {check.kind === 'checking' ? (
        <p className="cockpit-review__hint" role="status">
          正在核对现在生效的内容和二次认证状态…
        </p>
      ) : check.kind === 'failed' ? (
        <p className="cockpit-review__hint" role="alert">
          没能核对，预览没有展开：{check.message} 再按一次「批准并发布」重试。
        </p>
      ) : check.kind !== 'idle' ? (
        <p className="cockpit-review__hint" role="status">
          {VERDICT_TEXT[check.kind]}
        </p>
      ) : null}
      {open ? (
        <ActionPreview
          id={previewId}
          content={open.content}
          disabled={actions.busy}
          onConfirm={confirm}
          onCancel={actions.closePreview}
        />
      ) : null}
      <div className="cockpit-review__actions">
        <Button
          data-queue-approve=""
          aria-label={`${label}，批准并发布`}
          aria-expanded={approveOpen}
          aria-controls={approveOpen ? previewId : undefined}
          // 核对的那一小会儿不禁用（再按由 approve 自己挡下）：禁用会让按钮丢掉焦点，
          // 预览收起时焦点就还不回这个按钮了。
          disabled={actions.busy || blocker !== null}
          onClick={approve}
        >
          批准并发布
        </Button>
        {!desired.report || stale ? (
          <Button
            data-review-validate=""
            aria-label={`${label}，${desired.report ? '重新校验' : '校验'}`}
            disabled={actions.busy}
            onClick={validate}
          >
            {desired.report ? '重新校验' : '校验'}
          </Button>
        ) : null}
        <Button
          variant="text"
          data-queue-reject=""
          aria-label={`${label}，${verb}`}
          aria-expanded={rejectOpen}
          aria-controls={rejectOpen ? previewId : undefined}
          disabled={actions.busy}
          onClick={reject}
        >
          {verb}
        </Button>
      </div>
    </div>
  );
}
