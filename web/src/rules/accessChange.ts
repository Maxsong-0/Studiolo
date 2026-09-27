import type { DesiredRecord } from '../policyModel';
import { object, strings, withRuleAccess } from './rulesModel';
import type { TypeColumn } from './rulesModel';

/*
 * 权限矩阵切一格之后要做什么（T-160g）：以这一列现有的主人草案为底追加，改回生效原状则只撤回。
 * 从 rulesModel 拆出，守单文件 300 行上限。
 */

/** 切一格之后要做什么：交一份（新的或追加的）草案，或者改回了生效原状、只需撤回草案。 */
export type AccessChange =
  | { kind: 'submit'; body: string; replaces: DesiredRecord | null }
  | { kind: 'withdraw'; draft: DesiredRecord };

/**
 * 以这一列现有的主人草案为底（没有草案才用生效正文）改一格。改完若与生效正文等价（见 sameAccess），
 * 说明草案已无改动，只撤回它；否则交新草案，并注明要取代哪一份旧草案。
 */
export function accessChange(
  column: TypeColumn,
  rule: string,
  allowed: boolean,
): AccessChange {
  const draft = column.ownerDraft;
  const body = withRuleAccess(draft?.body ?? column.body ?? '', rule, allowed);
  if (draft && column.body && sameAccess(body, column.body))
    return { kind: 'withdraw', draft };
  return { kind: 'submit', body, replaces: draft };
}

/** 至少带一个 read 对象的能力集正文。 */
type Capability = Record<string, unknown> & { read: Record<string, unknown> };

/** 解析出带 read 的能力集对象；读不出返回 null。 */
function capabilityRoot(body: string): Capability | null {
  let root: unknown;
  try {
    root = JSON.parse(body);
  } catch {
    return null;
  }
  return object(root) && object(root.read) ? (root as Capability) : null;
}

/**
 * 两份能力集正文在读权限上是否等价：规则名单按集合比（顺序无意义；缺字段＝空名单，都是零规则），
 * 其余字段连同键顺序逐字比。先删后加同一条规则会把它排到名单末尾，逐字比较会误判为「有改动」。
 */
function sameAccess(left: string, right: string): boolean {
  const a = capabilityRoot(left);
  const b = capabilityRoot(right);
  if (!a || !b) return false;
  const rules = (root: Capability) => {
    const list = root.read.rules_whitelist;
    if (list === undefined) return '';
    return strings(list) ? [...list].sort().join('\n') : null;
  };
  const ruleA = rules(a);
  if (ruleA === null || ruleA !== rules(b)) return false;
  const rest = (root: Capability) => {
    const read = { ...root.read };
    delete read.rules_whitelist;
    return JSON.stringify({ ...root, read });
  };
  return rest(a) === rest(b);
}
