/**
 * 驾驶舱队列里就地批准、退回一份策略草案（T-200，纯函数）：能不能批、不能的原因、各种说法。
 * 写动作本身走全站那一份 policy/usePolicyActions；这里只做判断和措辞，不发请求。
 *
 * 批准用的是校验报告的基准摘要（`base_effective_hash`）：人在这一行里看到的扩大项是照哪一份生效内容算的，
 * 就按哪一份提交。校验之后生效内容变过，报告就旧了——不让批，先重新校验。
 */
import { newerDraftReason, withdrawVerb } from '../policy/previews';
import type { RecentAuth } from '../policy/recentAuth';
import { STATUS_LABEL, shortId } from '../policyModel';
import type {
  DesiredStatus,
  ListedDesired,
  PolicyStatus,
} from '../policyModel';
import { desiredTitle } from './queueText';

/** 就地审批的反馈落在哪一块：`review:<草案号>`。 */
export const REVIEW_ORIGIN = 'review:';
export const reviewOrigin = (id: string) => `${REVIEW_ORIGIN}${id}`;
/** 两个执行预览的 key；发布的那个与策略页同一写法。 */
export const publishKey = (id: string) => `publish:${id}`;
export const withdrawKey = (id: string) => `withdraw:${id}`;

/**
 * 对象的名字，与队列里那一行的标题同一个：「能力集 claude」「规则母本 CLAUDE.md」，
 * 下线申请是「下线能力集 claude」。按钮的读屏名字、结果那一句都用它，一行只有一个叫法。
 */
export function reviewLabel(desired: ListedDesired): string {
  return desiredTitle(desired);
}

/** 不用核对就知道这份草案在这里批准不了的原因；能批返回 null。 */
export function approveBlocker(desired: ListedDesired): string | null {
  const report = desired.report;
  if (!report) return '这份草案还没有校验报告：先校验。';
  if (!report.ok)
    return '校验没通过：去策略页看逐条问题，改好重交，或者在这里退回。';
  return newerDraftReason(desired.blocked_by);
}

/** 核对（读了现在生效的内容和二次认证状态）之后的结论。 */
export type ReviewVerdict =
  /** 可以展开执行预览。 */
  | { kind: 'ready' }
  /** 校验报告不是照现在生效的内容算的。 */
  | { kind: 'stale' }
  /** 本入口没有一次性验证码路径。 */
  | { kind: 'no-path' }
  /** 本页没有受控会话，没法就地认证。 */
  | { kind: 'no-session' };

export function reviewVerdict(
  desired: ListedDesired,
  status: PolicyStatus,
  auth: RecentAuth,
): ReviewVerdict {
  if (!status.l3_available || auth.kind === 'no-path')
    return { kind: 'no-path' };
  if (auth.kind === 'no-session') return { kind: 'no-session' };
  if (desired.report?.base_effective_hash !== status.effective_hash)
    return { kind: 'stale' };
  return { kind: 'ready' };
}

/** 核对没过时写在按钮上方的那一句。 */
export const VERDICT_TEXT: Record<
  Exclude<ReviewVerdict['kind'], 'ready'>,
  string
> = {
  stale:
    '这份草案校验之后，生效中的内容变过（比如刚发布了别的草案）：上面的扩大项是照旧内容算的，不能直接批准。先重新校验，看过新的结果再批准。',
  'no-path': '本入口没有一次性验证码路径：这里只能查看和退回，不能发布。',
  'no-session': '本页没有可用的受控会话，不能就地认证：请重新登录后再发布。',
};

/**
 * 草案的「指纹」：状态、校验时刻、校验基准、正文摘要。它变了，核对的结论和开着的预览
 * 说的就不是眼前这一份了。
 */
export function reviewSignature(desired: ListedDesired): string {
  const report = desired.report;
  return [
    desired.status,
    desired.validated_at ?? '',
    report?.base_effective_hash ?? '',
    report?.body_sha256 ?? '',
  ].join('|');
}

/** 撤回之后说的那一句：结果写在队列标题下面（那一行已经不在了），所以点明是哪一份。 */
export function withdrawnNotice(desired: ListedDesired): string {
  return `已${withdrawVerb(desired)} ${reviewLabel(desired)}（草案 ${shortId(desired.id)}）。`;
}

/**
 * 校验之后说的那一句。校验没通过的草案不算「等你点头」，会离开队列：结果写在队列标题下面，
 * 所以点明是哪一份、去了哪里。
 */
export function validatedNotice(
  desired: ListedDesired,
  status: DesiredStatus,
): string {
  const which = `${reviewLabel(desired)}（草案 ${shortId(desired.id)}）`;
  return status === 'rejected'
    ? `已重新校验 ${which}：校验未通过，它不再排在这里等你点头。去策略页的待处理草案里看逐条问题，改好重交。`
    : `已重新校验 ${which}：${STATUS_LABEL[status]}。`;
}
