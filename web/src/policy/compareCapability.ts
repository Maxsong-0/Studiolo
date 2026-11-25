/**
 * 能力集的「现在／改后」对比（T-198）。纯函数：两段正文（JSON 文本）进，一个字段一行出。
 *
 * 这张表只帮人看懂「改了哪几项能力」，不做判定：
 * - 名单（数组）按集合比：只看多了哪几项、少了哪几项，先后和重复不算改动。
 * - 一边整份正文都在、只是没写某个名单，按空名单算——后端对这几个名单的缺省就是空。
 * - 不认识的字段照样列一行，路径原样；任何一边读不出结构就整张表不出，说明原因，不猜。
 * - 哪些改动算「扩大」「收窄」只查服务端校验报告（{@link serverDirection}），这里不另判一套。
 */
import type { Expansion } from '../policyModel';
import { CAPABILITY_FIELDS } from './capabilityLabels';

export type JsonScalar = string | number | boolean | null;

/** 名单里的一项。`onlyHere`：另一边没有它——在「现在」那一列是去掉的，在「改后」那一列是新增的。 */
export interface ListEntry {
  text: string;
  onlyHere: boolean;
}

/** 表里的一格。`absent`：这一边没有这一项（整份都没有，或单值字段没写）。 */
export type CapabilityCell =
  | { kind: 'absent' }
  | { kind: 'scalar'; value: JsonScalar }
  | { kind: 'list'; items: readonly ListEntry[] };

export interface CapabilityRow {
  /** 字段路径，如 `read.project_whitelist`。 */
  path: string;
  changed: boolean;
  before: CapabilityCell;
  after: CapabilityCell;
}

/** 读不出结构的原因：正文是空的、不是完整的 JSON、是 JSON 但不是一个对象。 */
export type UnreadableReason = 'empty' | 'not_json' | 'not_object';

export type CapabilityComparison =
  | { kind: 'unreadable'; side: 'before' | 'after'; reason: UnreadableReason }
  | { kind: 'rows'; rows: readonly CapabilityRow[]; changed: number };

type Leaf =
  | { kind: 'scalar'; value: JsonScalar }
  | { kind: 'list'; items: readonly string[] };
type Fields = ReadonlyMap<string, Leaf>;
type Parsed =
  { ok: true; fields: Fields } | { ok: false; reason: UnreadableReason };

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** 键按字典序写出的 JSON：内容相同、键序不同的两个对象算同一项。 */
function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (isObject(value))
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
      .join(',')}}`;
  return JSON.stringify(value);
}

/**
 * 名单里一项的写法：字符串原样；硬绑定写成「触发条件 → skill」（与服务端校验报告里的写法逐字相同，
 * 这样才对得上它判的扩大、收窄）；其余写成 JSON。
 */
function entryText(item: unknown): string {
  if (typeof item === 'string') return item;
  if (
    isObject(item) &&
    Object.keys(item).length === 2 &&
    typeof item.when === 'string' &&
    typeof item.must_use === 'string'
  )
    return `${item.when} → ${item.must_use}`;
  return canonical(item);
}

/** 路径里的一段：寻常的键原样；带点号或别的符号的键加引号，免得和嵌套的路径撞在一起。 */
function segment(key: string): string {
  return /^[A-Za-z0-9_-]+$/.test(key) ? key : JSON.stringify(key);
}

function collect(
  value: Record<string, unknown>,
  prefix: string,
  into: Map<string, Leaf>,
): void {
  for (const [key, child] of Object.entries(value)) {
    const path = prefix ? `${prefix}.${segment(key)}` : segment(key);
    if (Array.isArray(child))
      into.set(path, {
        kind: 'list',
        items: [...new Set(child.map(entryText))],
      });
    else if (isObject(child) && Object.keys(child).length)
      collect(child, path, into);
    // 空对象没有可展开的字段：照实写成一格，不当它不存在。
    else if (isObject(child)) into.set(path, { kind: 'scalar', value: '{}' });
    else into.set(path, { kind: 'scalar', value: child as JsonScalar });
  }
}

/** 一边的正文。null 是「没有正文」（现在没有生效版本，或这是一次撤销）：读得出，只是一个字段都没有。 */
function parse(body: string | null): Parsed {
  if (body === null) return { ok: true, fields: new Map() };
  if (!body.trim()) return { ok: false, reason: 'empty' };
  let value: unknown;
  try {
    value = JSON.parse(body);
  } catch {
    return { ok: false, reason: 'not_json' };
  }
  if (!isObject(value)) return { ok: false, reason: 'not_object' };
  const fields = new Map<string, Leaf>();
  collect(value, '', fields);
  return { ok: true, fields };
}

/** 行序：认识的字段照 CAPABILITY_FIELDS 的顺序，其余按出现的先后（先「现在」后「改后」）。 */
function orderedPaths(before: Fields, after: Fields): string[] {
  const paths = new Set<string>();
  for (const field of CAPABILITY_FIELDS)
    if (before.has(field.path) || after.has(field.path)) paths.add(field.path);
  for (const path of before.keys()) paths.add(path);
  for (const path of after.keys()) paths.add(path);
  return [...paths];
}

function cell(
  leaf: Leaf | undefined,
  other: Leaf | undefined,
  sideAbsent: boolean,
): CapabilityCell {
  if (leaf === undefined)
    // 这一边整份都在、只是没写这个名单：按空名单算。
    return !sideAbsent && other?.kind === 'list'
      ? { kind: 'list', items: [] }
      : { kind: 'absent' };
  if (leaf.kind === 'scalar') return leaf;
  const others = new Set(other?.kind === 'list' ? other.items : []);
  return {
    kind: 'list',
    items: leaf.items.map((text) => ({ text, onlyHere: !others.has(text) })),
  };
}

function differs(before: CapabilityCell, after: CapabilityCell): boolean {
  if (before.kind === 'scalar' && after.kind === 'scalar')
    return before.value !== after.value;
  // 名单对名单（含一边没有）：任何一边有对方没有的项才算改了；没有对空名单不算。
  if (before.kind !== 'scalar' && after.kind !== 'scalar')
    return [before, after].some(
      (side) =>
        side.kind === 'list' && side.items.some((item) => item.onlyHere),
    );
  // 一边是单值：另一边是名单或没有，都算改了。
  return true;
}

/**
 * 比两段能力集正文。`before` 是对比基准（生效中那一版），null＝现在没有生效版本；
 * `after` 是草案正文，null＝这是一次撤销。
 */
export function compareCapability(
  before: string | null,
  after: string | null,
): CapabilityComparison {
  const left = parse(before);
  if (!left.ok)
    return { kind: 'unreadable', side: 'before', reason: left.reason };
  const right = parse(after);
  if (!right.ok)
    return { kind: 'unreadable', side: 'after', reason: right.reason };
  const rows = orderedPaths(left.fields, right.fields).map((path) => {
    const a = left.fields.get(path);
    const b = right.fields.get(path);
    const beforeCell = cell(a, b, before === null);
    const afterCell = cell(b, a, after === null);
    return {
      path,
      changed: differs(beforeCell, afterCell),
      before: beforeCell,
      after: afterCell,
    };
  });
  return {
    kind: 'rows',
    rows,
    changed: rows.filter((row) => row.changed).length,
  };
}

export type ChangeDirection = 'expansion' | 'narrowing';

/**
 * 服务端校验报告有没有把这一处改动判成扩大或收窄。只查表：报告里有这一条才标，没有就不标——
 * 比如新加的项目已经被名单里更宽的前缀盖住，服务端不算它扩大，这里也就不标。
 * `item` 是名单里的那一项（`side` 说它在哪一列）；单值字段传 null，只按路径查。
 */
export function serverDirection(
  report: {
    expansions: readonly Expansion[];
    narrowings: readonly Expansion[];
  },
  path: string,
  side: 'before' | 'after',
  item: string | null,
): ChangeDirection | null {
  const hit = (entry: Expansion) =>
    entry.path === path &&
    (item === null ||
      (side === 'before'
        ? entry.before === item && entry.after === null
        : entry.after === item && entry.before === null));
  if (report.expansions.some(hit)) return 'expansion';
  if (report.narrowings.some(hit)) return 'narrowing';
  return null;
}
