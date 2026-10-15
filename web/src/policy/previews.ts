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
  /** 红字：只给「等你逐条确认」的扩大项。 */
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

/**
 * 发布一份已校验的规则草案。`superseded` 是同一对象里较早的未发布草案：发布这一份时服务端把它们
 * 一并标为已取代（只有最新那份能发布，所以剩下的都比它早，T-181）。
 */
export function publishPreview(
  record: DesiredRecord,
  report: Report,
  status: PolicyStatus,
  superseded: readonly DesiredRecord[],
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
          ? `同一对象较早的 ${superseded.length} 份未发布草案（${superseded
              .map((entry) => shortId(entry.id))
              .join('、')}）：发布后标为已取代，不能再发布`
          : '无：同一对象没有别的未发布草案',
      },
    ],
    notes: [
      ...(report.retire ? ['这是一次撤销：发布后该对象不再有生效版本。'] : []),
      '不可撤回：发布后成为新的生效版本，版本历史只增不减；要改回只能再发布或回滚出新版本。',
      L3_NOTE('发布'),
    ],
    confirmLabel: '确认发布',
  };
}

/** 发布成功提示的补充：发布事务把同一对象剩下的未发布草案一并标成了已取代（执行预览里列过）。 */
export function supersededNotice(supersedes: number): string {
  return supersedes ? '；同一对象其余未发布的草案已一并标为已取代' : '';
}

/**
 * 这份草案不是同一对象最新的那份时，发布按钮为什么不可点（服务端会以 newer_desired_exists 拒绝）；
 * `newer` 是接口给的更新草案号。是最新的返回 null。
 */
export function newerDraftReason(newer: readonly string[]): string | null {
  if (!newer.length) return null;
  return `同一对象还有更新的未发布草案（${newer.map(shortId).join('、')}）：只能发布最新的那一份；要发这份，先在「策略」页的待处理草案里撤回更新的。`;
}

/** 发布这份最新草案会把同一对象较早的几份一并标为已取代：发布按钮旁先说一句。没有较早的返回 null。 */
export function supersededHint(count: number): string | null {
  return count
    ? `同一对象另有 ${count} 份较早的未发布草案：发布这一份会把它们标为已取代。`
    : null;
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
