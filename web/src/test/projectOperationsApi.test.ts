import { expect, it, vi } from 'vitest';
import { getProjectOperations } from '../api';
import { validateProjectOperations } from '../projectOperations';
import { installSessionReader } from '../sessionHttp';
import { operationsData } from './operationsFixtures';

it.each([
  {
    directories: Array.from({ length: 257 }, (_, index) => ({
      path: `目录${index}`,
      label: '职责',
      source: null,
    })),
  },
  {
    comparison_parents: Array.from(
      { length: 65 },
      (_, index) => `父目录${index}`,
    ),
  },
  {
    drift: Array.from({ length: 1281 }, (_, index) => ({
      path: `目录${index}`,
      status: 'unverified',
      source: null,
    })),
  },
  {
    drift: [
      { path: '重复', status: 'present', source: null },
      { path: '重复', status: 'missing', source: null },
    ],
  },
])('超出结构数据上限或漂移路径重复时拒绝渲染 %#', (override) => {
  expect(() =>
    validateProjectOperations({ ...operationsData(), ...override }),
  ).toThrow('数据格式不完整');
});

it('接受上限以内的结构数据', () => {
  const data = operationsData({
    directories: Array.from({ length: 256 }, (_, index) => ({
      path: `目录${index}`,
      label: '职责',
      source: null,
    })),
    comparison_parents: Array.from(
      { length: 64 },
      (_, index) => `父目录${index}`,
    ),
    drift: Array.from({ length: 1280 }, (_, index) => ({
      path: `目录${index}`,
      status: 'unverified',
      source: null,
    })),
  });
  expect(validateProjectOperations(data)).toBe(data);
});

it.each([
  { repository: { status: 'ready', observed_at: 'not-a-time' } },
  { repository: { status: 'ready', observed_at: '9007199254740992' } },
  { repository: { status: 'ready', observed_at: '-1' } },
  { repository: { status: 'ready', observed_at: '8640000000001' } },
  {
    directories: [
      { path: 'web', label: { html: '<b>label</b>' }, source: null },
    ],
  },
  {
    directories: [
      { path: 'web', label: '职责', source: { path: '文档.md', line: 0 } },
    ],
  },
  { drift: [{ path: 'web', status: 'unknown', source: null }] },
])('损坏契约字段拒绝进入图和观测时间展示 %#', (override) => {
  expect(() =>
    validateProjectOperations({ ...operationsData(), ...override }),
  ).toThrow('数据格式不完整');
});

it('项目运作沿用已安装的会话读取器', async () => {
  const sessionReader = vi.fn().mockResolvedValue(operationsData());
  const dispose = installSessionReader(sessionReader);
  const controller = new AbortController();
  try {
    const result = await getProjectOperations(
      '项目/星舟',
      7,
      controller.signal,
    );
    expect(result.project).toBe('项目/星舟');
    expect(sessionReader).toHaveBeenCalledWith(
      `/api/project-operations?${new URLSearchParams({ project: '项目/星舟', generation: '7' })}`,
      controller.signal,
    );
  } finally {
    dispose();
  }
});
