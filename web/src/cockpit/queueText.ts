/**
 * 「等你」队列里一行要说的话（纯函数）：标题、标题下的灰字、选中时多出来的一行事实、去处理的动作。
 * 只把已经在手的字段换成一句人话，不补数据；三种来源各说各的，说法跟各自的页面一致
 * （执行者名取 collab/executorGrouping，策略对象取 policyModel 的 KIND_LABEL）。
 */
import { agentName, executorOf } from '../collab/executorGrouping';
import { KIND_LABEL } from '../policyModel';
import type { ListedDesired } from '../policyModel';
import { actionTitle, type PendingEntry } from './pending';

export interface QueueText {
  title: string;
  /** 标题下面的灰字：属于哪个项目、谁交来的。 */
  subtitle: string;
  /** 选中时多出来的一行：一个等宽小标签，加几条事实。 */
  facts: { label: string; items: string[] };
  action: {
    /** 选中行主按钮上的字。 */
    primary: string;
    /** 其余行行尾文字链接上的字。 */
    link: string;
    /** 读屏名字：带上标题，一页里每个动作都叫得出是哪一件。 */
    name: string;
  };
}

/** 标签里实例号的位数：与执行者标签的起始位数一致（「Claude · 01M3」）。 */
const INSTANCE_PREFIX = 4;

/** 草案是谁交的：AI 写类型和实例号前几位（「Claude · 01M3 提交」），主人写「你提交」。 */
function submittedBy(desired: ListedDesired): string {
  if (desired.submitter_kind === 'owner') return '你提交';
  const name = desired.agent_type ? agentName(desired.agent_type) : 'AI';
  return desired.agent_instance
    ? `${name} · ${desired.agent_instance.slice(0, INSTANCE_PREFIX)} 提交`
    : `${name} 提交`;
}

/** 草案的标题：对象类型加名字；正文为空的是下线申请。 */
export function desiredTitle(desired: ListedDesired): string {
  const retire = desired.report?.retire ?? desired.body === null;
  return `${retire ? '下线' : ''}${KIND_LABEL[desired.kind]} ${desired.subject}`;
}

function desiredFacts(desired: ListedDesired): string[] {
  const report = desired.report;
  if (!report) return ['还没有校验报告'];
  const { diff } = report;
  return [
    report.ok ? '通过' : `没通过，${report.problems.length} 个问题`,
    report.expansions.length > 0
      ? `扩大 ${report.expansions.length} 项`
      : '没有扩大项',
    ...(report.narrowings.length > 0
      ? [`收窄 ${report.narrowings.length} 项`]
      : []),
    `改动 +${diff.added} −${diff.removed} ~${diff.changed}`,
    ...(desired.newer_open.length > 0
      ? [`同一对象还有 ${desired.newer_open.length} 份更新的草案`]
      : []),
  ];
}

/**
 * 一行的全部文字。`labels` 是整份黑板汇总的执行者显示名（executorGrouping 的 overviewLabels），
 * 黑板条目用它写「谁交来的」，与右栏、协作页同名。
 */
export function queueText(
  entry: PendingEntry,
  labels?: ReadonlyMap<string, string>,
): QueueText {
  switch (entry.source) {
    case 'board': {
      const { item } = entry;
      const identity = executorOf(item);
      const who = labels?.get(identity.key) ?? agentName(identity.agentType);
      const progress = item.progress;
      return {
        title: item.title,
        subtitle: `${entry.project} · ${who}`,
        // 状态已经写在类型一栏（待验收／待修订），这里只补进度，不换个说法再说一遍。
        facts: {
          label: '进度',
          items: [
            ...(progress
              ? [`工具调用 ${progress.tools} 次`, `改动 ${progress.edits} 处`]
              : ['还没有进度回报']),
            `占用 ${item.paths.length} 条路径`,
          ],
        },
        action: {
          primary: '去协作页处理',
          link: '去协作页',
          name: `${item.title}，在协作页查看`,
        },
      };
    }
    case 'policy': {
      const { desired } = entry;
      const title = desiredTitle(desired);
      return {
        title,
        subtitle: `${submittedBy(desired)} · ${
          desired.status === 'validated' ? '已校验，可发布' : '待复核'
        }`,
        facts: { label: '校验', items: desiredFacts(desired) },
        action: {
          primary: '去策略页处理',
          link: '去策略页',
          name: `${title}，在策略页处理`,
        },
      };
    }
    case 'inbox': {
      const { action } = entry;
      return {
        title: actionTitle(action),
        subtitle: entry.project,
        facts: {
          label: '出处',
          items: [`${action.source.path} 第 ${action.source.line} 行`],
        },
        action: {
          primary: '查看原文',
          link: '看原文',
          name: `${action.title}，查看原文`,
        },
      };
    }
  }
}
