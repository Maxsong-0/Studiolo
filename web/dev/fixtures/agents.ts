/** 接入方列表与黑板使用同一组实例、父关系及当前策略；仅在开发假数据模式加载。 */
import { INSTANCE, INSTANCE_TREE, lineage, type InstanceKey } from './ids.ts';
import { HOUR, iso } from './clock.ts';
import {
  activeRevocations,
  effectiveEntries,
  globalRevoked,
  type PolicyState,
} from './policy.ts';
export function agentsBody(policy: PolicyState, now: number) {
  const revocations = activeRevocations(policy),
    entries = effectiveEntries(policy);
  const revoked = (level: string, target: string) =>
    revocations.find((r) => r.level === level && r.target === target) ?? null;
  const types = ['claude', 'codex'].map((name) => ({
    name,
    issued_at: iso(now - 168 * HOUR),
    identity_revoked_at: null,
    capability_version:
      entries.find((e) => e.kind === 'capability_set' && e.subject === name)
        ?.version ?? null,
    policy_revocation: revoked('type', name),
  }));
  return {
    generated_at: iso(now),
    effective_hash: policy.effective,
    global_revoked: globalRevoked(policy),
    children_enabled: true,
    types,
    instances: (Object.keys(INSTANCE) as InstanceKey[])
      .map((key) => {
        const node = INSTANCE_TREE[key],
          id = INSTANCE[key];
        const own = revoked('instance', id),
          kind = types.find((t) => t.name === node.type)!;
        const ancestor = lineage(key)
          .slice(0, -1)
          .reverse()
          .find((ancestor) => revoked('instance', INSTANCE[ancestor]));
        const reason = globalRevoked(policy)
          ? 'global_revoked'
          : kind.policy_revocation
            ? 'type_revoked'
            : own
              ? 'instance_revoked'
              : kind.capability_version === null
                ? 'no_capability_set'
                : ancestor
                  ? 'ancestor_revoked'
                  : null;
        return {
          id,
          agent_type: node.type,
          parent: node.parent ? INSTANCE[node.parent] : null,
          registered_at: iso(now - (key === 'REVOKED' ? 72 : 8) * HOUR),
          last_seen_at: iso(now - (key === 'REVOKED' ? 48 : 0.5) * HOUR),
          denial: reason
            ? {
                reason,
                by:
                  reason === 'ancestor_revoked' && ancestor
                    ? INSTANCE[ancestor]
                    : null,
              }
            : null,
          policy_revocation: own,
        };
      })
      .sort(
        (a, b) =>
          b.registered_at.localeCompare(a.registered_at) ||
          b.id.localeCompare(a.id),
      ),
    instances_truncated: false,
  };
}
