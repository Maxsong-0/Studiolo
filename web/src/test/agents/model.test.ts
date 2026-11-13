import { describe, expect, it } from 'vitest';
import { agentsBody } from '../../../dev/fixtures/agents';
import { initialPolicy } from '../../../dev/fixtures/policy';
import { INSTANCE } from '../../../dev/fixtures/ids';
import {
  parseAgents,
  relativeCall,
  visibleInstances,
} from '../../agents/model';
const now = Date.parse('2026-11-13T12:00:00Z');
const body = () => agentsBody(initialPolicy(now), now);
describe('接入方封闭模型', () => {
  it('与开发黑板身份一致，24 小时筛选保留链接指向的旧实例', () => {
    const parsed = parseAgents(body());
    expect(parsed.instances.find((i) => i.id === INSTANCE.C1)?.parent).toBe(
      INSTANCE.R1,
    );
    expect(
      visibleInstances(parsed, true, null).some(
        (i) => i.id === INSTANCE.REVOKED,
      ),
    ).toBe(false);
    expect(
      visibleInstances(parsed, true, INSTANCE.REVOKED).some(
        (i) => i.id === INSTANCE.REVOKED,
      ),
    ).toBe(true);
    expect(relativeCall('2026-11-13T10:00:00Z', parsed.generated_at)).toBe(
      '2 小时前',
    );
  });
  it.each([
    (data: Record<string, unknown>) => {
      data.session_digest = 'secret';
    },
    (data: Record<string, unknown>) => {
      delete data.instances_truncated;
    },
    (data: Record<string, unknown>) => {
      data.effective_hash = 'bad';
    },
    (data: Record<string, unknown>) => {
      const rows = data.instances as Record<string, unknown>[];
      rows[0].denial = { reason: 'ancestor_revoked', by: null };
    },
    (data: Record<string, unknown>) => {
      const rows = data.instances as Record<string, unknown>[];
      rows[0].parent = rows[0].id;
    },
    (data: Record<string, unknown>) => {
      const rows = data.instances as Record<string, unknown>[];
      rows.push(rows[0]);
    },
  ])('拒绝缺字段、秘密字段、错误关联与不完整拒绝原因', (change) => {
    const data = body();
    change(data);
    expect(() => parseAgents(data)).toThrow();
  });
});
