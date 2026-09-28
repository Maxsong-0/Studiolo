/**
 * 规则视图的纯函数模型（T-160，ADR-026）。规则的唯一母本是策略平面 rules 类的生效版本（D1）；
 * 「哪类 AI 能读哪条规则」只看各类型能力集的 read.rules_whitelist，缺字段即零规则（D3、D4）。
 * 这里只做推导与改写，不发请求：所有结论都来自接口给的生效条目、版本正文与规则草案（desired）。
 */
import type {
  DesiredRecord,
  EffectiveEntry,
  PolicyStatus,
  VersionRecord,
} from '../policyModel';

// 规则名与正文的预校验（与后端同口径）单独成文件，这里一并导出，调用方只认规则模型这一个入口。
export {
  RULE_BODY_MAX_BYTES,
  RULE_SUBJECT_MAX_CHARS,
  bodyBytes,
  ruleSubjectProblem,
  validRuleSubject,
} from './ruleSubject';

/** 能力集里的规则读名单：listed＝列了哪些；unset＝缺字段（零规则）；invalid＝读不出，不猜。 */
export type Whitelist =
  | { kind: 'listed'; rules: readonly string[] }
  | { kind: 'unset' }
  | { kind: 'invalid' };

export function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export function strings(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((item) => typeof item === 'string')
  );
}

/** 从能力集正文读出 read.rules_whitelist。 */
export function ruleWhitelist(body: string | null): Whitelist {
  if (body === null) return { kind: 'invalid' };
  let root: unknown;
  try {
    root = JSON.parse(body);
  } catch {
    return { kind: 'invalid' };
  }
  if (!object(root) || !object(root.read)) return { kind: 'invalid' };
  if (!Object.hasOwn(root.read, 'rules_whitelist')) return { kind: 'unset' };
  const list = root.read.rules_whitelist;
  return strings(list) ? { kind: 'listed', rules: list } : { kind: 'invalid' };
}

/**
 * 把能力集正文的 read.rules_whitelist 改成「能读／不能读 rule」，产出新的 JSON 字符串。
 * 只动这一个字段：其余字段、值与键顺序原样保留（JSON.parse 保留键的插入顺序）；名单原有顺序也保留，
 * 新加的规则排在末尾；缺字段时在 read 的末尾补上。正文读不出或名单形状不对就抛错，绝不在猜出来的结构上改。
 */
export function withRuleAccess(
  body: string,
  rule: string,
  allowed: boolean,
): string {
  let root: unknown;
  try {
    root = JSON.parse(body);
  } catch {
    throw new Error('能力集正文不是合法的 JSON，不能改读权限。');
  }
  if (!object(root) || !object(root.read))
    throw new Error('能力集正文缺少 read，不能改读权限。');
  const read = root.read;
  const current = Object.hasOwn(read, 'rules_whitelist')
    ? read.rules_whitelist
    : [];
  if (!strings(current))
    throw new Error('能力集的 read.rules_whitelist 不是规则名列表，不能改。');
  read.rules_whitelist = allowed
    ? current.includes(rule)
      ? current
      : [...current, rule]
    : current.filter((item) => item !== rule);
  return JSON.stringify(root);
}

const ACTIONABLE: ReadonlySet<DesiredRecord['status']> = new Set([
  'pending',
  'validated',
  'rejected',
]);

/** 还没发布、仍可发布／重新校验／撤回的规则草案。 */
export function isOpenDraft(record: DesiredRecord): boolean {
  return ACTIONABLE.has(record.status);
}

function time(value: string): number {
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? 0 : parsed;
}

/** 新提交的在前；同一时刻按 id（ULID）倒序。 */
function newestFirst(left: DesiredRecord, right: DesiredRecord): number {
  const delta = time(right.submitted_at) - time(left.submitted_at);
  if (delta) return delta;
  return left.id < right.id ? 1 : left.id > right.id ? -1 : 0;
}

/** 码点序：与后端 SQLite 默认的字节序比较一致，排序在各处稳定。 */
function codeOrder(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

export interface RuleRow {
  subject: string;
  /** 生效条目；还没发布过（只有草案，或只出现在读名单里）的规则为 null。 */
  entry: EffectiveEntry | null;
  /** 生效正文；没有生效版本为 null。 */
  body: string | null;
  /** 能读这条规则的 AI 类型（能力集名单里有它），按类型名排序。 */
  readers: string[];
  /** 这条规则还没发布的草案，新的在前。 */
  drafts: DesiredRecord[];
}

export interface TypeColumn {
  /** AI 类型名（能力集的 subject）。 */
  type: string;
  entry: EffectiveEntry;
  /** 生效中的能力集正文；读不到为 null。 */
  body: string | null;
  whitelist: Whitelist;
  /**
   * 主人提交、尚未发布的最新一份能力集草案：矩阵这一列按它显示「待发布」，再改格子就在它上面追加
   * （交一份新草案、撤回它），不另起一份基于生效正文的草案，免得前面的改动被覆盖。
   */
  ownerDraft: DesiredRecord | null;
  /** 这份草案提议的名单；没有草案为 null。 */
  proposed: Whitelist | null;
  /** 同一类型其余未发布的能力集草案（多为 AI 提案）：发布本列的改动会把它们一并标为已取代。 */
  others: number;
  /** 该类型的撤销接入生效中。 */
  revoked: boolean;
}

export interface RulesModel {
  rules: RuleRow[];
  types: TypeColumn[];
  /** 能力集正文读不出名单的类型：读者统计不含它们，界面必须写明。 */
  unreadable: string[];
}

/** 由生效条目、各版本正文与草案列表推出规则行与读权限的列。 */
export function buildRulesModel(input: {
  status: PolicyStatus;
  desired: readonly DesiredRecord[];
  records: ReadonlyMap<number, VersionRecord>;
}): RulesModel {
  const { status, desired, records } = input;
  const open = desired.filter(isOpenDraft).sort(newestFirst);
  const bodyOf = (entry: EffectiveEntry) =>
    records.get(entry.version)?.body ?? null;
  const revoked = new Set(
    status.revocations
      .filter((row) => row.level === 'type' && row.released_at === null)
      .map((row) => row.target),
  );
  const types = status.entries
    .filter((entry) => entry.kind === 'capability_set')
    .sort((left, right) => codeOrder(left.subject, right.subject))
    .map((entry): TypeColumn => {
      const body = bodyOf(entry);
      const drafts = open.filter(
        (record) =>
          record.kind === 'capability_set' && record.subject === entry.subject,
      );
      const ownerDraft =
        drafts.find((record) => record.submitter_kind === 'owner') ?? null;
      return {
        type: entry.subject,
        entry,
        body,
        whitelist: ruleWhitelist(body),
        ownerDraft,
        proposed: ownerDraft ? ruleWhitelist(ownerDraft.body) : null,
        others: drafts.length - (ownerDraft ? 1 : 0),
        revoked: revoked.has(entry.subject),
      };
    });
  const readers = new Map<string, string[]>();
  for (const column of types) {
    if (column.whitelist.kind !== 'listed') continue;
    for (const rule of new Set(column.whitelist.rules))
      readers.set(rule, [...(readers.get(rule) ?? []), column.type]);
  }
  const effective = new Map(
    status.entries
      .filter((entry) => entry.kind === 'rules')
      .map((entry) => [entry.subject, entry]),
  );
  const subjects = new Set([
    ...effective.keys(),
    ...open
      .filter((record) => record.kind === 'rules')
      .map((record) => record.subject),
    ...readers.keys(),
  ]);
  const rules = [...subjects].sort(codeOrder).map((subject): RuleRow => {
    const entry = effective.get(subject) ?? null;
    return {
      subject,
      entry,
      body: entry ? bodyOf(entry) : null,
      readers: readers.get(subject) ?? [],
      drafts: open.filter(
        (record) => record.kind === 'rules' && record.subject === subject,
      ),
    };
  });
  return {
    rules,
    types,
    unreadable: types
      .filter((column) => column.whitelist.kind === 'invalid')
      .map((column) => column.type),
  };
}

/** 名单里有没有这条规则；读不出的名单返回 null（不猜）。 */
export function listsRule(whitelist: Whitelist, rule: string): boolean | null {
  if (whitelist.kind === 'invalid') return null;
  return whitelist.kind === 'listed' && whitelist.rules.includes(rule);
}

export interface AccessCell {
  /** 生效中能不能读；名单读不出为 null。 */
  effective: boolean | null;
  /** 矩阵里显示的值：有待发布草案时按草案，否则按生效。 */
  shown: boolean | null;
  /** 待发布草案把这一格改了（与生效不同）。 */
  pending: boolean;
  /** 这一列不能改：生效正文或待发布草案读不出名单（不在猜出来的结构上改）。 */
  locked: boolean;
}

export function accessCell(column: TypeColumn, rule: string): AccessCell {
  const effective = listsRule(column.whitelist, rule);
  const proposed = column.proposed ? listsRule(column.proposed, rule) : null;
  const shown = proposed ?? effective;
  return {
    effective,
    shown,
    pending: proposed !== null && proposed !== effective,
    locked:
      effective === null || !column.body || column.proposed?.kind === 'invalid',
  };
}
