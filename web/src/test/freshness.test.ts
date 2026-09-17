import { describe, expect, it } from 'vitest';
import { validFreshness } from '../freshness';
import { validateWorkbench } from '../validate';
import { fixture } from './fixtures';

const fresh = {
  stale: false,
  indexed_through: '18446744073709551615',
  committed_through: '18446744073709551615',
  unresolved: false,
  watcher_healthy: true,
  reason: null,
};

describe('canonical 新鲜度证据', () => {
  it('保留 u64 十进制水位，不转成有损 JS number', () => {
    const response = validateWorkbench({ ...fixture, freshness: fresh });
    expect(response.freshness).toEqual(fresh);
  });

  it('旧服务缺字段和只读副本 null 保持未知', () => {
    expect(validateWorkbench(fixture).freshness).toBeUndefined();
    expect(
      validateWorkbench({ ...fixture, freshness: null }).freshness,
    ).toBeNull();
  });

  it('陈旧观测允许未知水位和未决状态', () => {
    expect(
      validFreshness({
        ...fresh,
        stale: true,
        indexed_through: null,
        committed_through: null,
        unresolved: null,
        watcher_healthy: false,
        reason: 'watcher_failed',
      }),
    ).toBe(true);
  });

  it.each([
    undefined,
    [],
    {},
    false,
    'fresh',
    { ...fresh, stale: 'false' },
    { ...fresh, watcher_healthy: 1 },
    { ...fresh, unresolved: [] },
    { ...fresh, reason: ['changed'] },
    { ...fresh, reason: '<script>' },
    { ...fresh, stale: true, reason: 'x'.repeat(65) },
    { ...fresh, committed_through: 1 },
    { ...fresh, committed_through: '01' },
    { ...fresh, committed_through: '-1' },
    { ...fresh, committed_through: '1e3' },
    { ...fresh, committed_through: '18446744073709551616' },
    { ...fresh, committed_through: '1'.repeat(100) },
    { ...fresh, indexed_through: null },
    { ...fresh, indexed_through: '1' },
    { ...fresh, unresolved: true },
    { ...fresh, unresolved: null },
    { ...fresh, watcher_healthy: false },
    { ...fresh, reason: 'changed' },
  ])('拒绝 present 畸形或自相矛盾的响应 %j', (value) => {
    expect(() => validateWorkbench({ ...fixture, freshness: value })).toThrow(
      '工作台数据格式不完整',
    );
  });
});
