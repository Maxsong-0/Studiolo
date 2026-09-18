import { expect, it, vi, afterEach } from 'vitest';
import { NO_RECOVERY_POINT, parseHealth, readHealth } from '../healthApi';

function body(overrides: Record<string, unknown> = {}) {
  return {
    overall: 'normal',
    evidence_complete: true,
    sections: [
      {
        code: 'recovery-point',
        title: '恢复点年龄',
        state: 'normal',
        detail: '最新恢复点 100 秒前的内容，已在预算内',
      },
    ],
    ...overrides,
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

it('证据齐备的响应被解析为整体正常且证据完整', () => {
  const report = parseHealth(body());
  expect(report.overall).toBe('normal');
  expect(report.evidenceComplete).toBe(true);
  expect(report.sections).toHaveLength(1);
});

it('未建立恢复点的响应保留既定中文且整体不显示正常', () => {
  const report = parseHealth(
    body({
      overall: 'unknown',
      evidence_complete: false,
      sections: [
        {
          code: 'recovery-point',
          title: '恢复点年龄',
          state: 'unknown',
          detail: NO_RECOVERY_POINT,
        },
      ],
    }),
  );
  expect(report.overall).not.toBe('normal');
  expect(report.sections[0].detail).toBe(NO_RECOVERY_POINT);
});

it('无法识别的状态值一律降到未知而不是当作正常', () => {
  const report = parseHealth(
    body({
      overall: 'green',
      sections: [
        {
          code: 'disk-quota',
          title: '磁盘配额',
          state: 'ok',
          detail: '在预算内',
        },
      ],
    }),
  );
  expect(report.overall).toBe('unknown');
  expect(report.sections[0].state).toBe('unknown');
});

it('后端自称证据完整但仍有未知节时前端不接受完整标记', () => {
  const report = parseHealth(
    body({
      evidence_complete: true,
      sections: [
        {
          code: 'disk-quota',
          title: '磁盘配额',
          state: 'unknown',
          detail: '读数不可得',
        },
      ],
    }),
  );
  expect(report.evidenceComplete).toBe(false);
});

it('响应完全不是对象或缺少检查项时整体判未知', () => {
  expect(parseHealth(null).overall).toBe('unknown');
  expect(parseHealth('normal').overall).toBe('unknown');
  expect(parseHealth(body({ sections: [] })).overall).toBe('unknown');
  expect(parseHealth(body({ sections: 'normal' })).sections).toHaveLength(0);
});

it('检查项缺少机器码时被丢弃而不是补一个空壳正常项', () => {
  const report = parseHealth(
    body({
      sections: [{ title: '磁盘配额', state: 'normal', detail: '在预算内' }],
    }),
  );
  expect(report.sections).toHaveLength(0);
  expect(report.overall).toBe('unknown');
});

it('检查项缺少正文时以证据缺失兜底而不是留空', () => {
  const report = parseHealth(
    body({ sections: [{ code: 'disk-quota', state: 'normal' }] }),
  );
  expect(report.sections[0].detail).toBe('证据缺失。');
  expect(report.sections[0].title).toBe('disk-quota');
});

it('读取健康页走固定的鉴权 API 路径而不是另开入口', async () => {
  const seen: string[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (path: string) => {
      seen.push(path);
      return new Response(JSON.stringify(body()), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }),
  );

  const report = await readHealth(new AbortController().signal);

  expect(seen).toEqual(['/api/v1/health']);
  expect(report.overall).toBe('normal');
});

it('后端报错时读取抛出而不是把失败洗成正常报告', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(
      async () =>
        new Response(JSON.stringify({ error: '本机视图暂不可用，请重试。' }), {
          status: 503,
          headers: { 'Content-Type': 'application/json' },
        }),
    ),
  );

  await expect(readHealth(new AbortController().signal)).rejects.toThrow(
    '本机视图暂不可用',
  );
});

it('一次失败之后再次读取成功可以恢复出正常报告', async () => {
  let call = 0;
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => {
      call += 1;
      return call === 1
        ? new Response(JSON.stringify({ error: '暂不可用' }), { status: 503 })
        : new Response(JSON.stringify(body()), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          });
    }),
  );

  await expect(readHealth(new AbortController().signal)).rejects.toThrow();
  const report = await readHealth(new AbortController().signal);

  expect(report.overall).toBe('normal');
});
