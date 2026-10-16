import type {
  DesiredRecord,
  PolicyStatus,
  Report,
  RevokeRequest,
  VersionMeta,
} from '../policyModel';
import { KIND_LABEL, LEVEL_LABEL, shortHash, shortId } from '../policyModel';

/**
 * 不可逆动作「执行预览」的文字（纯函数，设计规范第 8、9 节）：做什么、对哪个对象、
 * 从哪个版本到哪个版本或撤销范围、能不能撤回、要不要一次性验证码。
 * 只写接口已经给出的事实：拿不到的（如回滚会带来哪些扩大项）写「—」和原因，不猜。
 * 措辞依据 docs/contracts/policy-plane.md 第 3、4 节。
 */

export interface PreviewFact {
  label: string;
  value: string;
  /** 红字：只给等你处理的事实，即要逐条确认的扩大项、发布后仍等你处理的更新草案。 */
  signal?: boolean;
}

export interface PreviewContent {
  /** 框里第一行：做什么。 */
  title: string;
  facts: PreviewFact[];
  /** 能不能撤回、要不要一次性验证码，各一句。 */
  notes: string[];
  /** 框里确认按钮的名字（只用人话）。 */
  confirmLabel: string;
}

const L3_NOTE = (verb: string) =>
  `需要二次认证（一次性验证码）：服务端要求时会弹出窗口请你输入，通过后才会${verb}。`;

/** 请求带的 expected_effective_hash：人看过哪一份就按哪一份提交。 */
function basis(effectiveHash: string): PreviewFact {
  return {
    label: '依据',
    value: `生效中摘要 ${shortHash(effectiveHash)}；期间若有别的发布，服务端会挡下这一次`,
  };
}

/** 该对象眼下生效的是哪一版：取刚读到的生效中各项（发布与回滚同一口径）。 */
function currentVersion(
  status: PolicyStatus,
  kind: DesiredRecord['kind'],
  subject: string,
): string {
  const entry = status.entries.find(
    (row) => row.kind === kind && row.subject === subject,
  );
  return entry ? `当前生效 v${entry.version}` : '当前没有生效版本';
}

/** 几份草案的号（前 12 位），顿号隔开。 */
function draftIds(records: readonly DesiredRecord[]): string {
  return records.map((entry) => shortId(entry.id)).join('、');
}

/** 谁交的：主人，或哪一类 AI（同一类 AI 的几个实例算同一提交者）。 */
function submitterOf(record: DesiredRecord): string {
  return record.submitter_kind === 'agent'
    ? `AI ${record.agent_type ?? '未知类型'}`
    : '主人';
}

/**
 * 发布一份已校验的草案（T-181）。`superseded` 是同一对象里比它早的未发布草案：发布时服务端把它们一并
 * 标为已取代。`remaining` 是比它更新的：同一提交者更新的会挡住发布，所以能走到这里的只会是别的提交者
 * 交的，发布后照旧待处理（用户 2026-10-16 定：谁的提议都不悄悄作废）。
 */
export function publishPreview(
  record: DesiredRecord,
  report: Report,
  status: PolicyStatus,
  superseded: readonly DesiredRecord[],
  remaining: readonly DesiredRecord[],
): PreviewContent {
  const target = `${KIND_LABEL[record.kind]} ${record.subject}`;
  const from = currentVersion(status, record.kind, record.subject);
  const expansions = report.expansions.length;
  return {
    title: `发布 ${target}`,
    facts: [
      { label: '对象', value: target },
      // 发的就是这一份：草案号写全，与队列里的号逐位对得上。
      { label: '草案', value: record.id },
      {
        label: '版本',
        value: `${from} → 新版本（编号在发布时由服务端分配）`,
      },
      basis(status.effective_hash),
      {
        label: '扩大项',
        value: expansions ? `${expansions} 项，见上方校验报告` : '无',
        signal: expansions > 0,
      },
      {
        label: '同时取代',
        value: superseded.length
          ? `同一对象比它早的 ${superseded.length} 份未发布草案（${draftIds(
              superseded,
            )}）：发布后标为已取代，不能再发布`
          : '无：同一对象没有比它早的未发布草案',
      },
      ...(remaining.length
        ? [
            {
              label: '仍待处理',
              value: `同一对象还有 ${remaining.length} 份比它更新的未发布草案，是别的提交者交的（${remaining
                .map((entry) => `${shortId(entry.id)}，${submitterOf(entry)}`)
                .join(
                  '；',
                )}）：这次发的是较早的这一份，它们不会被取代，发布后照旧等你处理`,
              signal: true,
            },
          ]
        : []),
    ],
    notes: [
      ...(report.retire ? ['这是一次撤销：发布后该对象不再有生效版本。'] : []),
      '不可撤回：发布后成为新的生效版本，版本历史只增不减；要改回只能再发布或回滚出新版本。',
      L3_NOTE('发布'),
    ],
    // 还有更新的草案时，确认按钮也点明发的是较早的这一份，不让人当成在发最新的。
    confirmLabel: remaining.length ? '确认发布这份较早的草案' : '确认发布',
  };
}

/**
 * 发布成功提示的补充（执行预览里列过）：比它早的已一并标为已取代，比它更新的照旧待处理。
 * 份数是确认时的；之后又有变化，以重读后的列表为准。
 */
export function publishedNotice(supersedes: number, remains: number): string {
  return `${supersedes ? '；同一对象比它早的未发布草案已一并标为已取代' : ''}${
    remains ? `；同一对象更新的 ${remains} 份未发布草案照旧待处理` : ''
  }`;
}

/**
 * 同一提交者（主人，或同一类 AI）还有更新的未发布草案时，发布按钮为什么不可点（服务端会以
 * newer_desired_same_submitter 拒绝）；`blocked` 是接口给的挡住它的那几份。没被挡返回 null。
 */
export function newerDraftReason(blocked: readonly string[]): string | null {
  if (!blocked.length) return null;
  return `同一提交者还有更新的未发布草案（${blocked.map(shortId).join('、')}）：只能发布其中最新的那一份；要发这份，先在「策略」页的待处理草案里撤回更新的。`;
}

/**
 * 发布按钮旁先说一句：发布这一份会取代几份比它早的、留下几份比它更新的（别的提交者交的）。
 * 都没有返回 null。
 */
export function publishHint(
  superseded: number,
  remaining: number,
): string | null {
  const parts = [
    ...(superseded
      ? [
          `同一对象另有 ${superseded} 份较早的未发布草案：发布这一份会把它们标为已取代。`,
        ]
      : []),
    ...(remaining
      ? [
          `同一对象还有 ${remaining} 份更新的未发布草案，是别的提交者交的：发布这一份不会取代它们，它们照旧待处理。`,
        ]
      : []),
  ];
  return parts.length ? parts.join('') : null;
}

/** 回滚到某个历史版本：新建一个正文等于它的版本，历史不变。 */
export function rollbackPreview(
  meta: VersionMeta,
  status: PolicyStatus,
): PreviewContent {
  const target = `${KIND_LABEL[meta.kind]} ${meta.subject}`;
  const from = currentVersion(status, meta.kind, meta.subject);
  return {
    title: `回滚到 v${meta.version}`,
    facts: [
      { label: '对象', value: target },
      {
        label: '版本',
        value: `${from} → 新版本，正文等于 v${meta.version}`,
      },
      basis(status.effective_hash),
      {
        label: '扩大项',
        value: '—（回滚前拿不到：服务端回滚时按当前生效内容重新校验判定）',
      },
    ],
    notes: [
      ...(meta.retired
        ? [`v${meta.version} 是撤销版本：回滚后该对象不再有生效版本。`]
        : []),
      '不可撤回：回滚是新建一个版本，历史不变；要改回只能再发布或再回滚。',
      L3_NOTE('回滚'),
    ],
    confirmLabel: '确认回滚',
  };
}

/** 解除全局撤销接入。 */
export function releasePreview(effectiveHash: string): PreviewContent {
  return {
    title: '解除全局撤销',
    facts: [
      { label: '范围', value: '全局：所有 AI 类型与实例' },
      {
        label: '结果',
        value: '恢复按各自生效中的能力集接入；实例级与类型级的撤销照旧生效',
      },
      basis(effectiveHash),
    ],
    notes: [
      '解除即刻生效，不可撤回；要再拦住全部 AI，只能重新做一次全局撤销接入。',
      L3_NOTE('解除'),
    ],
    confirmLabel: '确认解除',
  };
}

/** 三级撤销接入：即刻生效，本身不走二次认证。 */
export function revokePreview(request: RevokeRequest): PreviewContent {
  const target = request.target ?? '';
  const scope =
    request.level === 'instance'
      ? `实例 ${target}：只拒绝这一个实例，同类型的其他实例不受影响`
      : request.level === 'type'
        ? `类型 ${target}：该类型的全部实例，连同新的握手`
        : '全局：所有 AI 的全部调用';
  const undo =
    request.level === 'instance'
      ? '不可撤回：实例级撤销不能恢复。'
      : request.level === 'type'
        ? '不可撤回；恢复只能重新发布该类型的能力集（需要二次认证）。'
        : '不可撤回；恢复只能走「解除全局撤销」（需要二次认证）。';
  return {
    title: `撤销接入：${LEVEL_LABEL[request.level]}${target ? ` ${target}` : ''}`,
    facts: [
      { label: '范围', value: scope },
      { label: '理由', value: request.reason },
      { label: '生效', value: '确认后即刻生效，下一次调用就被拒绝' },
    ],
    notes: [undo, '撤销接入本身不需要一次性验证码：确认后立即提交。'],
    confirmLabel: '确认撤销接入',
  };
}
