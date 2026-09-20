import { expect, it } from 'vitest';
import { boardItem } from './boardModel';
import {
  executionStatus,
  candidatePatch,
  landingCommand,
  landingResult,
  replayPage,
  reviewReceipt,
  reviewTemplate,
  startCommand,
} from './executionModel';
import {
  executionBoard,
  executionEvent,
  executionHash,
  executionId,
  executionProject,
  executionReceipt,
  executionRun,
} from './test/executionFixtures';

const landingHead = 'b'.repeat(40);
const landingRequest = {
  operation: 'land' as const,
  task_id: executionId(),
  expected_head: landingHead,
  patch_sha256: executionHash,
};
const landed = {
  task_id: executionId(),
  attempt_id: `A-${executionId(101)}`,
  patch_sha256: executionHash,
  expected_head: landingHead,
  after_tree: 'c'.repeat(40),
  applied: true,
  replayed: false,
  conflicts: [],
};
it('landing accepts precise success, replay and conflict results with candidate-bound identities', () => {
  expect(landingResult(landed, landingRequest)).toEqual(landed);
  expect(
    landingResult({ ...landed, replayed: true }, landingRequest).replayed,
  ).toBe(true);
  expect(
    landingResult(
      { ...landed, applied: false, conflicts: ['src/app.ts'] },
      landingRequest,
    ),
  ).toMatchObject({ applied: false, conflicts: ['src/app.ts'] });
  const head64 = 'b'.repeat(64);
  expect(
    landingResult(
      { ...landed, expected_head: head64, after_tree: 'c'.repeat(64) },
      { ...landingRequest, expected_head: head64 },
    ).expected_head,
  ).toBe(head64);
});
it('landing rejects malformed, contradictory or unbound responses and host paths', () => {
  for (const change of [
    { task_id: executionId(2) },
    { attempt_id: 'invalid' },
    { expected_head: 'd'.repeat(40) },
    { patch_sha256: 'd'.repeat(64) },
    { after_tree: 'short' },
    { applied: 'true' },
    { replayed: 1 },
    { applied: false, replayed: true },
    { conflicts: ['src/app.ts'] },
    { applied: false, conflicts: ['/host/path'] },
    { applied: false, conflicts: ['../escape'] },
    { applied: false, conflicts: ['src/../escape'] },
    { applied: false, conflicts: ['C:\\host\\path'] },
    { applied: false, conflicts: ['src/a', 'src/a'] },
    { applied: false, conflicts: [4] },
    { unlanded_patch: '/host/candidate.patch' },
    { conflicts: undefined },
  ])
    expect(() =>
      landingResult({ ...landed, ...change }, landingRequest),
    ).toThrow();
});
it('landing command requires a single sealed patch, exact digest and an explicit full HEAD', () => {
  const task = boardItem(executionBoard('needs_review'));
  const patch = {
    path: `${executionProject}/03-记忆/candidate.patch`,
    revision: executionHash,
    sha256: executionHash,
  };
  task.candidateReceipt!.artifacts = [patch];
  expect(candidatePatch(task)).toEqual(patch);
  expect(landingCommand(task, landingHead, executionHash)).toEqual(
    landingRequest,
  );
  for (const change of [
    { status: 'running' as const },
    { attempt: null },
    { candidateReceipt: null },
    {
      candidateReceipt: {
        ...task.candidateReceipt!,
        artifacts: [patch, { ...patch, path: 'another/candidate.patch' }],
      },
    },
  ])
    expect(() =>
      landingCommand({ ...task, ...change }, landingHead, executionHash),
    ).toThrow();
  expect(() => landingCommand(task, '', executionHash)).toThrow(/HEAD/);
  expect(() => landingCommand(task, 'HEAD', executionHash)).toThrow(/HEAD/);
  expect(() => landingCommand(task, landingHead, 'd'.repeat(64))).toThrow(
    /SHA-256/,
  );
});

it('accepts only actual execution states and budget phases; unconfigured must be frozen and empty', () => {
  expect(
    executionStatus({ configured: false, frozen: true, runs: [] }),
  ).toEqual({ configured: false, frozen: true, runs: [] });
  for (const state of [
    'running',
    'candidate_review',
    'candidate_retained',
    'stopped',
    'failed',
  ])
    expect(
      executionStatus({
        configured: true,
        frozen: false,
        runs: [executionRun(state)],
      }).runs[0].state,
    ).toBe(state);
  for (const input of [
    { configured: false, frozen: false, runs: [] },
    { configured: false, frozen: true, runs: [executionRun()] },
    { configured: true, frozen: false, runs: [executionRun('completed')] },
    {
      configured: true,
      frozen: false,
      runs: [
        {
          ...executionRun(),
          progress: { tool_calls: -1, edits: 0, phase: 'working' },
        },
      ],
    },
    {
      configured: true,
      frozen: false,
      runs: [
        {
          ...executionRun(),
          progress: { tool_calls: 1, edits: 0, phase: 'almost' },
        },
      ],
    },
    { configured: true, frozen: false, runs: [executionRun(), executionRun()] },
    { configured: true, frozen: false, runs: [], raw: 'never accept' },
  ])
    expect(() => executionStatus(input)).toThrow();
});
it('candidate stages and committed candidates retain exact pointers and reject unknown schema', () => {
  const view = executionBoard('needs_review');
  view.candidate_stages = [
    {
      attempt_id: `A-${executionId(99)}`,
      fence: 1,
      pointer: { id: executionHash, sha256: executionHash },
      reason: 'older candidate retained',
    },
  ];
  const task = boardItem(view, executionProject);
  expect(task).toMatchObject({
    adapter: 'codex',
    candidateStages: [{ attemptId: `A-${executionId(99)}`, fence: 1 }],
    candidateReceipt: { manifestRevision: executionHash },
  });
  for (const change of [
    { candidate_stages: undefined },
    { candidate_receipt: undefined },
    {
      candidate_stages: [
        {
          attempt_id: `A-${executionId(99)}`,
          fence: 0,
          pointer: { id: executionHash, sha256: executionHash },
          reason: 'bad',
        },
      ],
    },
    {
      candidate_stages: [
        {
          attempt_id: `A-${executionId(99)}`,
          fence: 1,
          pointer: { id: executionHash, sha256: executionHash, raw: 'secret' },
          reason: 'bad',
        },
      ],
    },
    {
      candidate_receipt: {
        manifest_path: '../escape',
        manifest_revision: executionHash,
        artifacts: [],
      },
    },
    {
      candidate_receipt: {
        manifest_path: 'safe/a.json',
        manifest_revision: executionHash,
        artifacts: [
          { path: 'safe/a.md', revision: executionHash, sha256: 'bad' },
        ],
      },
    },
  ])
    expect(() => boardItem({ ...view, ...change })).toThrow();
});
it('start uses the task adapter and registered creator; missing or unsupported identities are rejected', () => {
  const task = boardItem(executionBoard());
  expect(startCommand(task)).toEqual({
    operation: 'start',
    task_id: executionId(),
    agent_type: 'codex',
    instance_id: 'registered-codex-instance',
  });
  for (const change of [
    { creator: null },
    { adapter: 'consumer' as const },
    { status: 'needs_review' as const },
  ])
    expect(() => startCommand({ ...task, ...change })).toThrow();
});
it('review never defaults to pass or implicit closeout and preserves explicit receipt', () => {
  const task = boardItem(executionBoard('needs_review'));
  const template = JSON.parse(reviewTemplate(task));
  expect(template.acceptance[0].result).toBe('unverified');
  expect(template.closeout.adr_required).toBeNull();
  expect(() => reviewReceipt(reviewTemplate(task), task)).toThrow();
  const receipt = executionReceipt();
  expect(reviewReceipt(JSON.stringify(receipt), task)).toEqual(receipt);
  expect(() =>
    reviewReceipt(
      JSON.stringify({
        ...receipt,
        acceptance: [
          { criterion: 'different criterion', result: 'pass', evidence: [] },
        ],
      }),
      task,
    ),
  ).toThrow();
  expect(() =>
    reviewReceipt(
      JSON.stringify({
        ...receipt,
        acceptance: [{ ...receipt.acceptance[0], result: 'unverified' }],
      }),
      task,
    ),
  ).toThrow();
});
it('replay preserves u64 cursors and accepts an isolated child snapshot', () => {
  const child = executionBoard();
  child.task.spec.parent = executionId(2);
  child.board.definition.parent = executionId(2);
  child.board.depth = 1;
  const page = replayPage(
    { task: child, events: [executionEvent('9007199254740993')], next: null },
    executionId(),
    executionProject,
    '9007199254740992',
  );
  expect(page.events[0].offset).toBe('9007199254740993');
  const events = Array.from({ length: 100 }, (_, i) =>
    executionEvent(String(i + 1)),
  );
  expect(
    replayPage(
      { task: child, events, next: '100' },
      executionId(),
      executionProject,
      null,
    ).next,
  ).toBe('100');
});
it('replay rejects raw fields, wrong task, duplicate offsets and invalid pagination', () => {
  const base = {
    task: executionBoard(),
    events: [executionEvent()],
    next: null,
  };
  for (const value of [
    { ...base, events: [{ ...executionEvent(), raw: 'body' }] },
    { ...base, events: [{ ...executionEvent(), details: { raw: 'body' } }] },
    {
      ...base,
      events: [
        { ...executionEvent(), details: { reason: { nested: 'body' } } },
      ],
    },
    { ...base, events: [executionEvent(), executionEvent()] },
    { ...base, events: [executionEvent('18446744073709551616')] },
    { ...base, events: [{ ...executionEvent(), phase: 'done' }] },
    { ...base, events: [{ ...executionEvent(), at: 'yesterday' }] },
    { ...base, next: '1' },
    {
      ...base,
      events: Array.from({ length: 101 }, (_, i) =>
        executionEvent(String(i + 1)),
      ),
    },
  ])
    expect(() =>
      replayPage(value, executionId(), executionProject, null),
    ).toThrow();
  expect(() =>
    replayPage(base, executionId(2), executionProject, null),
  ).toThrow();
});
