/**
 * 执行者：一条黑板任务归到谁名下（T-130 驾驶舱时间线与动态共用）。
 *
 * 分组键是持有者实例链的根（agent 类型＋根实例完整 id），没有持有者时退回任务的 adapter。
 * 显示名按任务书取「类型 · id 前 4 位」；但实例 id 是 ULID，前几位是时间戳——同几天里建的
 * 两条执行线前 4 位必然相同。所以只把前缀当显示名，分组仍用完整 id；同类型里前 4 位撞了，
 * 就把这一类的前缀统一加长到互不相同为止，免得两条线挤进一条泳道、或两条泳道同名。
 */
import type { BoardOverviewItem } from '../boardOverviewModel';

export interface Executor {
  /** 分组键：`类型:根实例 id`，无持有者时 `adapter:类型`。 */
  key: string;
  /** agent 类型或 adapter 原文，如 `claude`。 */
  agent: string;
  /** 根实例完整 id；无持有者时为 null。 */
  rootId: string | null;
}

/** 显示名前缀的最短长度（任务书：id 前 4 位）。 */
const PREFIX = 4;

export function executorOf(item: BoardOverviewItem): Executor {
  const root = item.holderLineage[0];
  return root
    ? {
        key: `${root.agentType}:${root.id}`,
        agent: root.agentType,
        rootId: root.id,
      }
    : { key: `adapter:${item.adapter}`, agent: item.adapter, rootId: null };
}

/** `claude` → `Claude`；只动首字母，其余原样。 */
function agentName(agent: string): string {
  return agent.charAt(0).toUpperCase() + agent.slice(1);
}

/** 同一类型里让所有根 id 前缀互不相同的最短长度（至少 4）。 */
function uniquePrefix(ids: readonly string[]): number {
  const longest = Math.max(PREFIX, ...ids.map((id) => id.length));
  for (let length = PREFIX; length < longest; length += 1)
    if (new Set(ids.map((id) => id.slice(0, length))).size === ids.length)
      return length;
  return longest;
}

/** 给一组执行者起显示名：`Claude · 01M3`；无持有者的只写类型。返回 键 → 显示名。 */
export function executorLabels(
  executors: Iterable<Executor>,
): Map<string, string> {
  const unique = new Map<string, Executor>();
  for (const executor of executors) unique.set(executor.key, executor);
  const byAgent = new Map<string, string[]>();
  for (const executor of unique.values())
    if (executor.rootId !== null)
      byAgent.set(executor.agent, [
        ...(byAgent.get(executor.agent) ?? []),
        executor.rootId,
      ]);
  const lengths = new Map(
    [...byAgent].map(([agent, ids]) => [agent, uniquePrefix(ids)]),
  );
  const labels = new Map<string, string>();
  for (const executor of unique.values())
    labels.set(
      executor.key,
      executor.rootId === null
        ? agentName(executor.agent)
        : `${agentName(executor.agent)} · ${executor.rootId.slice(
            0,
            lengths.get(executor.agent) ?? PREFIX,
          )}`,
    );
  return labels;
}
