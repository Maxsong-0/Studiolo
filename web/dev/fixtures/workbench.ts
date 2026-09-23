/**
 * `/api/workbench` 首屏：项目、待我动作、计数与新鲜度。
 *
 * 不手写一份与笔记脱节的清单，而是按后端 `crates/workbench/src/derive.rs` 的同一套规则从假 Vault
 * 派生：生命周期只认六值闭集（闭集外记诊断、值为 null）、进行中任务数来自 TASKS 的 ① 表、
 * 下一里程碑来自 roadmap、未关闭 BUG 来自 06-BUG 的 frontmatter，待我动作来自 INDEX 的
 * 「待我动作」小节，类型按后端 `descriptive_kind` 归到 待重启/待外部方/待验收/待拍板/待用户操作。
 */
import { PROJECT_NAMES, lineOf, root } from './vault.ts';
import type { VaultNote } from './vault.ts';

type Location = { path: string; line: number };
type Sourced<T> = { value: T | null; source: Location | null };

const LIFECYCLES = ['活跃', '暂停', '已交付维护', '静态资料', '候选', '未建档'];
const COMPLETED = [
  '已完成',
  '完成',
  '全部完成',
  'done',
  '已修复',
  '已关闭',
  '已取消',
  '已废弃',
  '已被取代',
];
const unknown = <T>(): Sourced<T> => ({ value: null, source: null });
const completed = (status: string) =>
  COMPLETED.some((prefix) => status.trim().toLowerCase().startsWith(prefix));

/** 后端 `actions::descriptive_kind` 的同一套判定。 */
export function descriptiveKind(text: string): string {
  if (text.includes('重启') || text.startsWith('done-pending-restart'))
    return '待重启';
  if (text.includes('外部方') || text.includes('第三方')) return '待外部方';
  if (text.includes('验收')) return '待验收';
  if (['拍板', '确认', '决策', '评审'].some((word) => text.includes(word)))
    return '待拍板';
  return '待用户操作';
}

function field(note: VaultNote, label: string) {
  const lines = note.content.split('\n');
  const index = lines.findIndex((line) => line.startsWith(`- ${label}：`));
  if (index < 0) return null;
  return {
    value: lines[index].slice(label.length + 3).trim(),
    line: index + 1,
  };
}

/** Markdown 表格数据行（跳过表头与分隔行），带 1 起算的行号。 */
function tableRows(note: VaultNote) {
  return note.content
    .split('\n')
    .map((text, index) => ({ text, line: index + 1 }))
    .filter(
      ({ text }) =>
        text.startsWith('| ') &&
        !text.startsWith('|---') &&
        !/^\| (任务|里程碑|目录|节点|编号|内容|层) \|/.test(text),
    )
    .map(({ text, line }) => ({
      cells: text
        .slice(1, -1)
        .split('|')
        .map((cell) => cell.trim()),
      line,
    }));
}

export interface WorkbenchParts {
  projects: unknown[];
  inbox: {
    id: string;
    project_id: string;
    project_name: string;
    kind: string;
    title: string;
    source: Location;
  }[];
  documents: number;
}

export function deriveWorkbench(notes: VaultNote[]): WorkbenchParts {
  const byPath = new Map(notes.map((note) => [note.path, note]));
  const projects: unknown[] = [];
  const inbox: WorkbenchParts['inbox'] = [];
  for (const name of PROJECT_NAMES) {
    const base = root(name);
    const diagnostics: string[] = [];
    const indexPath = `${base}/00-总览/INDEX.md`;
    const index = byPath.get(indexPath);
    const tasks = byPath.get(`${base}/04-协调/TASKS.md`);
    const roadmap = byPath.get(`${base}/05-版本/roadmap.md`);
    const declared = [index, tasks, roadmap].filter(Boolean).length;
    let lifecycle = unknown<string>();
    let version = unknown<string>();
    if (index) {
      const declaredLifecycle = field(index, '生命周期');
      if (declaredLifecycle && LIFECYCLES.includes(declaredLifecycle.value))
        lifecycle = {
          value: declaredLifecycle.value,
          source: { path: indexPath, line: declaredLifecycle.line },
        };
      else if (declaredLifecycle)
        diagnostics.push(
          `${indexPath}:${declaredLifecycle.line}: 生命周期不在闭集内`,
        );
      const declaredVersion = field(index, '当前版本');
      if (declaredVersion)
        version = {
          value: declaredVersion.value,
          source: { path: indexPath, line: declaredVersion.line },
        };
      inbox.push(...todos(name, index));
    } else diagnostics.push('缺少 00-总览/INDEX.md');
    if (lifecycle.value === null)
      diagnostics.push('生命周期未知：没有有效的明确声明');
    if (version.value === null)
      diagnostics.push('当前版本未知：没有有效的明确声明');
    let activeTasks = unknown<number>();
    if (tasks) {
      const rows = tableRows(tasks);
      activeTasks = {
        value: rows.filter((row) => !completed(row.cells[1] ?? '')).length,
        source: {
          path: tasks.path,
          line: lineOf(tasks.content, '任务 | 状态'),
        },
      };
    } else diagnostics.push('进行中任务数未知：缺少 04-协调/TASKS.md');
    let nextMilestone = unknown<string>();
    if (roadmap) {
      const next = tableRows(roadmap).find(
        (row) => !completed(row.cells[1] ?? ''),
      );
      if (next)
        nextMilestone = {
          value: next.cells[0],
          source: { path: roadmap.path, line: next.line },
        };
      else diagnostics.push(`${roadmap.path}: 没有可确认的下一里程碑`);
    } else diagnostics.push('下一里程碑未知：缺少 05-版本/roadmap.md');
    const bugs = notes
      .filter((note) => note.path.startsWith(`${base}/06-BUG/`))
      .sort((left, right) => (left.path < right.path ? -1 : 1));
    let openBugs = unknown<number>();
    if (bugs.length)
      openBugs = {
        value: bugs.filter((note) => !note.content.includes('status: fixed'))
          .length,
        source: {
          path: bugs[0].path,
          line: lineOf(bugs[0].content, 'status:'),
        },
      };
    else diagnostics.push('未关闭 BUG 数未知：没有可核验的 BUG 状态来源');
    projects.push({
      id: base,
      name,
      root: base,
      lifecycle,
      version,
      active_tasks: activeTasks,
      open_bugs: openBugs,
      next_milestone: nextMilestone,
      profile:
        declared === 0 ? 'unfiled' : declared === 3 ? 'standard' : 'partial',
      source: index ? { path: indexPath, line: 1 } : null,
      diagnostics,
    });
  }
  inbox.sort((left, right) =>
    compare(
      [left.project_id, left.source.path, left.source.line, left.kind],
      [right.project_id, right.source.path, right.source.line, right.kind],
    ),
  );
  return {
    projects,
    inbox,
    documents: notes.filter((note) => note.path.endsWith('.md')).length,
  };
}

function compare(left: (string | number)[], right: (string | number)[]) {
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] < right[index]) return -1;
    if (left[index] > right[index]) return 1;
  }
  return 0;
}

/** INDEX「待我动作」小节里未勾选的列表项；ID 与后端一致：`路径:行号:类型`。 */
function todos(name: string, index: VaultNote): WorkbenchParts['inbox'] {
  const lines = index.content.split('\n');
  const start = lines.indexOf('## 待我动作');
  if (start < 0) return [];
  const items: WorkbenchParts['inbox'] = [];
  for (let offset = start + 1; offset < lines.length; offset += 1) {
    const line = lines[offset];
    if (line.startsWith('## ')) break;
    if (!line.startsWith('- [ ] ')) continue;
    const title = line.slice(6).trim();
    const kind = descriptiveKind(title);
    const source = { path: index.path, line: offset + 1 };
    items.push({
      id: `${source.path}:${source.line}:${kind}`,
      project_id: root(name),
      project_name: name,
      kind,
      title,
      source,
    });
  }
  return items;
}

/** 计数：任一项目不可得就整体不可得，并逐个列出是哪个项目，绝不部分求和。 */
export function rollup(values: [string, number | null][]) {
  const unavailable = values
    .filter(([, value]) => value === null)
    .map(([id]) => id)
    .sort();
  return unavailable.length
    ? { value: null, unavailable }
    : {
        value: values.reduce((sum, [, value]) => sum + (value ?? 0), 0),
        unavailable: [],
      };
}

export const known = (value: number) => ({ value, unavailable: [] });

/** 「同步偏慢」：已观察到内容变化，索引水位落后提交水位三条，快照尚未重新核验。 */
export function freshness(fresh: boolean, committed: number) {
  return fresh
    ? {
        stale: false,
        indexed_through: String(committed),
        committed_through: String(committed),
        unresolved: false,
        watcher_healthy: true,
        reason: null,
      }
    : {
        stale: true,
        indexed_through: String(committed - 3),
        committed_through: String(committed),
        unresolved: false,
        watcher_healthy: true,
        reason: 'changed',
      };
}

export const SOURCE_LABEL = '假数据模式 · 只读副本';
