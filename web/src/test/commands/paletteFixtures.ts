import { vi } from 'vitest';
import type { CommandContext } from '../../commands/registry';
import { defaultViewLocation } from '../../viewLocation';
import type { Project } from '../../types';
import { project } from '../fixtures';

/** 动作面板测试用的上下文：动作全是间谍，数据按需覆盖。 */
export function paletteContext(overrides: Partial<CommandContext> = {}) {
  const actions = {
    goTo: vi.fn<CommandContext['goTo']>(),
    viewAllProjects: vi.fn<CommandContext['viewAllProjects']>(),
    refreshSnapshot: vi.fn<CommandContext['refreshSnapshot']>(),
    openSource: vi.fn<CommandContext['openSource']>(),
    openDrafts: vi.fn<CommandContext['openDrafts']>(),
    openRecentlyDeleted: vi.fn<CommandContext['openRecentlyDeleted']>(),
    openStatus: vi.fn<CommandContext['openStatus']>(),
    focusTarget: vi.fn<CommandContext['focusTarget']>(),
  };
  const context: CommandContext = {
    location: defaultViewLocation,
    projects: [project],
    overview: null,
    draftCount: 0,
    fileOperations: null,
    ...actions,
    ...overrides,
  };
  return { context, actions };
}

/** n 个项目，root 与 id 各不相同；第 i 个（从 0 起）没有来源时传进 `withoutSource`。 */
export function manyProjects(
  count: number,
  withoutSource: readonly number[] = [],
): Project[] {
  return Array.from({ length: count }, (_, index) => ({
    ...project,
    id: `p${String(index).padStart(2, '0')}`,
    name: `项目${String(index).padStart(2, '0')}`,
    root: `项目/项目${String(index).padStart(2, '0')}`,
    source: withoutSource.includes(index)
      ? null
      : {
          path: `项目/项目${String(index).padStart(2, '0')}/00-总览/INDEX.md`,
          line: 1,
        },
  }));
}

/** Crockford Base32 的 26 位任务 id（首位 0–7），尾号区分。 */
export function taskId(suffix: string): string {
  return `01J${'0'.repeat(23 - suffix.length)}${suffix}`;
}

interface OverviewItemInput {
  id: string;
  title: string;
  project: string;
  status: 'queued' | 'needs_review' | 'succeeded' | 'draft';
}

/**
 * 一份能过封闭解码器的黑板汇总（接口原样的 snake_case）：未结束项不带尝试，
 * 已结束项各配一条窗口内事件；items 按（项目, id）码点序排好。
 */
export function overviewResponse(
  items: readonly OverviewItemInput[],
  { truncated = false }: { truncated?: boolean } = {},
) {
  const to = '2026-09-22T12:00:00Z';
  const sorted = [...items].sort((left, right) =>
    left.project === right.project
      ? left.id < right.id
        ? -1
        : 1
      : left.project < right.project
        ? -1
        : 1,
  );
  return {
    generated_at: to,
    window_from: '2026-09-21T12:00:00Z',
    window_to: to,
    policy_hash: 'a'.repeat(64),
    items: sorted.map((item) => ({
      id: item.id,
      title: item.title,
      project: item.project,
      status: item.status,
      parent: null,
      adapter: 'claude',
      holder: null,
      holder_lineage: [],
      paths: ['web'],
      attempt: null,
      progress: null,
      receipt_status: null,
    })),
    items_truncated: truncated,
    events: sorted
      .filter((item) => item.status === 'succeeded')
      .map((item) => ({
        at: '2026-09-22T11:00:00Z',
        project: item.project,
        task_id: item.id,
        action: 'complete',
        phase: 'outcome',
        attempt_id: null,
      })),
    events_truncated: false,
  };
}
