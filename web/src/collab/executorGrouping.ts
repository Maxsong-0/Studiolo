/*
 * 执行者分组（协作拓扑与「今天的时间线」共用，纯函数）。
 * 认领过的任务按持有者实例链的根节点归组：同一根派生出来的子实例算同一个执行者；
 * 从未认领的任务（没有持有者）按执行器 adapter 归组。
 */
import type { BoardOverviewItem } from '../boardOverviewModel';

export interface ExecutorIdentity {
  /** 分组键：`instance:<根实例 id>`，从未认领时 `adapter:<adapter>`。 */
  key: string;
  /** 根实例的 agent 类型；从未认领时是 adapter。 */
  agentType: string;
  /** 根实例 id；从未认领时为 null。 */
  rootId: string | null;
}

export interface ExecutorGroup extends ExecutorIdentity {
  label: string;
  /** 组内任务，按任务 id 升序。 */
  items: BoardOverviewItem[];
}

/** 标签里 id 前缀的起始位数（「Claude · 01M3」）。 */
export const LABEL_PREFIX = 4;

const names: Record<string, string> = {
  claude: 'Claude',
  codex: 'Codex',
  consumer: 'Consumer',
  stub: 'Stub',
};

/** agent 类型的显示名：已知类型用规范写法，其余首字母大写、原样保留。 */
export function agentName(type: string): string {
  return (
    names[type.toLowerCase()] ??
    `${type.charAt(0).toUpperCase()}${type.slice(1)}`
  );
}

/** 短 id：只做显示，完整 id 另放在 title 或详情里。 */
export function shortId(id: string, length = 8): string {
  return id.slice(0, length);
}

export function executorOf(item: BoardOverviewItem): ExecutorIdentity {
  const root = item.holder === null ? undefined : item.holderLineage[0];
  if (root)
    return {
      key: `instance:${root.id}`,
      agentType: root.agentType,
      rootId: root.id,
    };
  return {
    key: `adapter:${item.adapter}`,
    agentType: item.adapter,
    rootId: null,
  };
}

const byText = (left: string, right: string) =>
  left < right ? -1 : left > right ? 1 : 0;

/**
 * 执行者标签：认领过的是「agent 名 · 根实例 id 前 4 位」，从未认领的是 adapter 名。
 * ULID 的前几位是时间戳，同一时段建的根实例前 4 位常常相同：出现同名时，
 * 所有认领标签一起逐位加长前缀，直到互不相同（统一位数，便于对齐比较）。
 */
export function executorLabels(
  identities: readonly ExecutorIdentity[],
): Map<string, string> {
  const unique = [
    ...new Map(identities.map((identity) => [identity.key, identity])).values(),
  ];
  const instances = unique.filter((identity) => identity.rootId !== null);
  const tag = (identity: ExecutorIdentity, length: number) =>
    `${agentName(identity.agentType)} · ${(identity.rootId ?? '').slice(0, length)}`;
  const longest = Math.max(
    LABEL_PREFIX,
    ...instances.map((identity) => (identity.rootId ?? '').length),
  );
  let length = LABEL_PREFIX;
  while (
    length < longest &&
    new Set(instances.map((identity) => tag(identity, length))).size <
      instances.length
  )
    length += 1;
  return new Map(
    unique.map((identity) => [
      identity.key,
      identity.rootId === null
        ? agentName(identity.agentType)
        : tag(identity, length),
    ]),
  );
}

/**
 * 按执行者分组。组序：认领过的按根实例 id 升序（ULID 即创建先后），从未认领的按 adapter 名排在后面；
 * 组内按任务 id 升序，结果与输入顺序无关。
 */
export function groupByExecutor(
  items: readonly BoardOverviewItem[],
  labels?: ReadonlyMap<string, string>,
): ExecutorGroup[] {
  const groups = new Map<string, ExecutorGroup>();
  const identities = items.map(executorOf);
  const named = labels ?? executorLabels(identities);
  items.forEach((item, index) => {
    const identity = identities[index];
    const group = groups.get(identity.key);
    if (group) group.items.push(item);
    else
      groups.set(identity.key, {
        ...identity,
        label: named.get(identity.key) ?? agentName(identity.agentType),
        items: [item],
      });
  });
  for (const group of groups.values())
    group.items.sort((left, right) => byText(left.id, right.id));
  return [...groups.values()].sort((left, right) => {
    if ((left.rootId === null) !== (right.rootId === null))
      return left.rootId === null ? 1 : -1;
    return byText(
      left.rootId ?? left.agentType,
      right.rootId ?? right.agentType,
    );
  });
}
