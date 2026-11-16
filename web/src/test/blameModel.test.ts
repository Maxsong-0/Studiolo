import { webcrypto } from 'node:crypto';
import { expect, it, vi } from 'vitest';
import { blame, blameMark, sha256Hex, type BlameRun } from '../blameModel';

const hex = (c: string) => c.repeat(64);
const plain = {
  history_id: null,
  revision: null,
  author: null,
  authored_at_ms: null,
};
const agent = {
  kind: 'agent',
  agent_type: 'claude',
  instance_id: 'i-1',
  task_id: 'T-1',
  attempt_id: null,
};
const good = () => ({
  vault_uuid: '00000000-0000-4000-8000-000000000001',
  path: 'a.md',
  revision: hex('a'),
  content_sha256: hex('b'),
  lines: 4,
  versions: 2,
  more_history: false,
  runs: [
    { from_line: 1, to_line: 2, origin: 'beyond_history', ...plain },
    {
      from_line: 3,
      to_line: 3,
      origin: 'version',
      history_id: hex('c'),
      revision: hex('d'),
      author: agent,
      authored_at_ms: 1_700_000_000_000,
    },
    { from_line: 4, to_line: 4, origin: 'untracked', ...plain },
  ],
});

it('归属各段必须从第 1 行起首尾相接、盖满全部行', () => {
  expect(blame(good(), 'a.md').runs).toHaveLength(3);
  const broken: ((value: ReturnType<typeof good>) => unknown)[] = [
    (v) => ({ ...v, lines: 5 }),
    (v) => ({ ...v, runs: v.runs.slice(1) }),
    (v) => ({ ...v, runs: [...v.runs, v.runs[2]] }),
    (v) => ({ ...v, extra: 1 }),
    (v) => ({ ...v, content_sha256: 'x' }),
    (v) => ({ ...v, runs: [{ ...v.runs[0], to_line: 0 }, ...v.runs.slice(1)] }),
    // 不是「某一版」的段不许带作者或版本；「某一版」的段必须带版本。
    (v) => ({
      ...v,
      runs: [{ ...v.runs[0], author: agent }, ...v.runs.slice(1)],
    }),
    (v) => ({
      ...v,
      runs: [v.runs[0], { ...v.runs[1], revision: null }, v.runs[2]],
    }),
    (v) => ({
      ...v,
      runs: [{ ...v.runs[0], origin: 'guess' }, ...v.runs.slice(1)],
    }),
    (v) => ({ ...v, runs: [{ ...v.runs[0], note: '' }, ...v.runs.slice(1)] }),
  ];
  for (const change of broken)
    expect(() => blame(change(good()), 'a.md')).toThrow('逐行归属响应格式无效');
  expect(() => blame(good(), 'b.md')).toThrow('逐行归属响应格式无效');
});

it('窄带上的缩写和整句：服务端说不清的照实说不清', () => {
  const run = (patch: Partial<BlameRun>): BlameRun => ({
    from_line: 3,
    to_line: 5,
    origin: 'version',
    history_id: hex('c'),
    revision: hex('d'),
    author: null,
    authored_at_ms: null,
    ...patch,
  });
  expect(blameMark(run({}))).toMatchObject({ kind: 'unknown', short: '?' });
  expect(blameMark(run({})).title).toContain('第 3–5 行：这一版的作者没有记录');
  expect(blameMark(run({})).title).toContain('版本 dddddddd · 时间未知');
  const ai = blameMark(run({ author: agent as BlameRun['author'] }));
  expect(ai).toMatchObject({ kind: 'agent', short: 'AI' });
  expect(ai.title).toContain('AI claude（i-1） · 任务 T-1');
  expect(blameMark(run({ author: { kind: 'obsidian' } }))).toMatchObject({
    kind: 'obsidian',
    short: 'Ob',
  });
  const human = blameMark(
    run({
      from_line: 7,
      to_line: 7,
      author: { kind: 'human', issuer: 'https://issuer', subject: '主人' },
    }),
  );
  expect(human).toMatchObject({ kind: 'human', short: '人' });
  expect(human.title).toContain('第 7 行：人 主人');
  expect(
    blameMark(run({ origin: 'untracked', revision: null, history_id: null })),
  ).toMatchObject({ kind: 'untracked', short: '外' });
  expect(
    blameMark(
      run({ origin: 'beyond_history', revision: null, history_id: null }),
    ),
  ).toMatchObject({ kind: 'beyond', short: '早' });
});

it('正文摘要是普通 SHA-256', async () => {
  if (!globalThis.crypto?.subtle) vi.stubGlobal('crypto', webcrypto);
  expect(await sha256Hex('')).toBe(
    'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
  );
  expect(await sha256Hex('一\n')).toMatch(/^[0-9a-f]{64}$/);
});
