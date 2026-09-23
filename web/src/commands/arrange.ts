/**
 * ⌘K 面板的排列：按输入过滤命令清单，分成「动作」「跳转」两组（文档组来自搜索结果），
 * 同类实体每类最多列 {@link FAMILY_LIMIT} 条，超出时补一行带总数的「全部 N …」入口，绝不静默截断
 * （设计规范第 5 节「项目入口不许静默截断」）。
 */
import type { SearchHit } from '../types';
import { highlightRanges, matchCommand, queryTerms } from './match';
import type { MatchRange } from './match';
import { documentCommand } from './registry';
import type { Command, CommandFamily, CommandGroup } from './registry';

const FAMILIES: readonly CommandFamily[] = ['project', 'source', 'task'];
export const FAMILY_LIMIT = 8;

export interface PaletteEntry {
  command: Command;
  /** 名字里要标红线的片段。 */
  ranges: readonly MatchRange[];
}

export interface PaletteSection {
  group: CommandGroup;
  entries: PaletteEntry[];
}

/** 某一类超出 8 条时补在末尾的「全部 N …」入口；N 是这一类的总数，来自接口。 */
function overflowCommand(
  family: CommandFamily,
  total: number,
  matched: number,
  tasksTruncated: boolean,
): Command {
  const shown =
    matched < total
      ? `匹配 ${matched} 条，这里列前 ${FAMILY_LIMIT} 条`
      : `这里列前 ${FAMILY_LIMIT} 条`;
  const base = { group: 'navigate' as const, keywords: [], hint: shown };
  if (family === 'project')
    return {
      ...base,
      id: 'more:project',
      label: `全部 ${total} 个项目`,
      run: (ctx) => ctx.viewAllProjects(),
    };
  if (family === 'source')
    return {
      ...base,
      id: 'more:source',
      label: `全部 ${total} 个项目来源`,
      hint: `${shown}；项目视图逐个列出来源`,
      run: (ctx) => ctx.viewAllProjects(),
    };
  return {
    ...base,
    id: 'more:task',
    label: tasksTruncated
      ? `全部黑板任务（汇总只取到 ${total} 条）`
      : `全部 ${total} 条黑板任务`,
    hint: `${shown}；去协作拓扑看全部`,
    target: () => ({ view: 'collab' }),
  };
}

/**
 * 按输入过滤并排好「动作」「跳转」两组（文档组由搜索结果另行给出）：
 * 组内先列非实体命令，再按项目、项目来源、黑板任务分类；同类按匹配度、再按清单顺序。
 */
export function arrangeCommands(
  commands: readonly Command[],
  query: string,
  { tasksTruncated = false }: { tasksTruncated?: boolean } = {},
): PaletteSection[] {
  const terms = queryTerms(query);
  const matched = commands.flatMap((command, order) => {
    const match = matchCommand(command, terms);
    return match ? [{ command, order, ...match }] : [];
  });
  const byScore = (list: typeof matched) =>
    [...list].sort(
      (left, right) => left.score - right.score || left.order - right.order,
    );
  const sections: PaletteSection[] = [];
  for (const group of ['action', 'navigate'] as const) {
    const inGroup = matched.filter((entry) => entry.command.group === group);
    const entries: PaletteEntry[] = byScore(
      inGroup.filter((entry) => !entry.command.family),
    );
    for (const family of FAMILIES) {
      const members = byScore(
        inGroup.filter((entry) => entry.command.family === family),
      );
      entries.push(...members.slice(0, FAMILY_LIMIT));
      if (members.length > FAMILY_LIMIT) {
        const total = commands.filter(
          (command) => command.family === family,
        ).length;
        entries.push({
          command: overflowCommand(
            family,
            total,
            members.length,
            tasksTruncated,
          ),
          ranges: [],
        });
      }
    }
    if (entries.length) sections.push({ group, entries });
  }
  return sections;
}

/** 文档组：命中的标题同样按输入标红线。 */
export function documentEntries(
  hits: readonly SearchHit[],
  query: string,
): PaletteEntry[] {
  const terms = queryTerms(query);
  return hits.map((hit, index) => ({
    command: documentCommand(hit, index),
    ranges: highlightRanges(hit.title, terms),
  }));
}
