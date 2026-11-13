import { describe, expect, it } from 'vitest';
import { INSTANCE, TASK } from '../../../dev/fixtures/ids';
import type { OverviewState } from '../../cockpit/model';
import { planPending, type PendingEntry } from '../../cockpit/pending';
import { desiredTitle, queueText } from '../../cockpit/queueText';
import { executorOf, overviewLabels } from '../../collab/executorGrouping';
import type { ListedDesired } from '../../policyModel';
import { fixture } from '../fixtures';
import { fixtureData, policyReady } from './cockpitFixtures';

const data = fixtureData();
const ready: OverviewState = { kind: 'ready', overview: data.overview };
const labels = overviewLabels(data.overview);
const [proposal] = data.desired;
const { entries } = planPending({
  board: ready,
  policy: policyReady(data.desired),
  inbox: fixture.inbox,
  projects: data.workbench.projects,
});
const board = (id: string) =>
  entries.find((entry) => entry.source === 'board' && entry.item.id === id)!;
const policyEntry = (desired: ListedDesired): PendingEntry => ({
  source: 'policy',
  n: 1,
  desired,
  kind: 'AI 提案',
  since: null,
  waitMs: null,
});

describe('队列里一行的文字：黑板任务', () => {
  it('标题是任务标题，灰字写项目和执行者（与协作页同名），选中时多一行进度', () => {
    const entry = board(TASK.T106);
    const item = entry.source === 'board' ? entry.item : null;
    expect(queueText(entry, labels)).toEqual({
      title: 'T-106 安装文档补运行包冻结',
      subtitle: `Studiolo · ${labels.get(executorOf(item!).key)}`,
      facts: {
        label: '进度',
        items: [
          '工具调用 88 次',
          '改动 19 处',
          `占用 ${item!.paths.length} 条路径`,
        ],
      },
      action: {
        primary: '去协作页处理',
        link: '去协作页',
        name: 'T-106 安装文档补运行包冻结，在协作页查看',
      },
    });
  });

  it('没给整份汇总的名字表时退回只写类型；没有进度回报时照实说', () => {
    const entry = board(TASK.T093);
    if (entry.source !== 'board') throw new Error('应是黑板条目');
    expect(queueText(entry).subtitle).toBe('Studiolo · Codex');
    const silent: PendingEntry = {
      ...entry,
      item: { ...entry.item, progress: null, paths: [] },
    };
    expect(queueText(silent, labels).facts).toEqual({
      label: '进度',
      items: ['还没有进度回报', '占用 0 条路径'],
    });
  });
});

describe('队列里一行的文字：策略里的提案和草案', () => {
  it('AI 提案：标题是对象类型加名字，灰字写谁交的和状态，选中时多一行校验结果', () => {
    const report = proposal.report!;
    const text = queueText(policyEntry(proposal));
    expect(text.title).toBe('能力集 claude');
    expect(text.subtitle).toBe(
      `Claude · ${INSTANCE.C3.slice(0, 4)} 提交 · 待复核`,
    );
    expect(text.facts.label).toBe('校验');
    expect(text.facts.items).toEqual([
      report.ok ? '通过' : `没通过，${report.problems.length} 个问题`,
      `扩大 ${report.expansions.length} 项`,
      ...(report.narrowings.length > 0
        ? [`收窄 ${report.narrowings.length} 项`]
        : []),
      `改动 +${report.diff.added} −${report.diff.removed} ~${report.diff.changed}`,
    ]);
    // 假数据这份提案扩大了三项读取范围。
    expect(report.expansions).toHaveLength(3);
    expect(text.action).toEqual({
      primary: '去策略页处理',
      link: '去策略页',
      name: '能力集 claude，在策略页处理',
    });
  });

  it('你自己交的、已校验的草案：写「你提交」和「已校验，可发布」', () => {
    const own: ListedDesired = {
      ...proposal,
      kind: 'rules',
      subject: 'CLAUDE.md',
      submitter_kind: 'owner',
      agent_type: null,
      agent_instance: null,
      status: 'validated',
    };
    const text = queueText(policyEntry(own));
    expect(text.title).toBe('规则母本 CLAUDE.md');
    expect(text.subtitle).toBe('你提交 · 已校验，可发布');
  });

  it('AI 没带类型或实例号时不编：只写知道的那部分', () => {
    expect(
      queueText(policyEntry({ ...proposal, agent_instance: null })).subtitle,
    ).toBe('Claude 提交 · 待复核');
    expect(
      queueText(
        policyEntry({ ...proposal, agent_type: null, agent_instance: null }),
      ).subtitle,
    ).toBe('AI 提交 · 待复核');
  });

  it('下线申请、没有校验报告、校验没通过、同一对象还有更新的草案，各自照实写', () => {
    const report = proposal.report!;
    expect(desiredTitle({ ...proposal, body: null, report: null })).toBe(
      '下线能力集 claude',
    );
    expect(
      desiredTitle({ ...proposal, report: { ...report, retire: true } }),
    ).toBe('下线能力集 claude');
    expect(
      queueText(policyEntry({ ...proposal, report: null })).facts.items,
    ).toEqual(['还没有校验报告']);
    const failing = queueText(
      policyEntry({
        ...proposal,
        newer_open: ['01M33ZA2G0HQ8K8HMTV4NH3KKS'],
        report: {
          ...report,
          ok: false,
          problems: [
            { path: 'read.project_whitelist[0]', message: '项目不存在' },
            { path: 'write', message: '不认识的字段' },
          ],
          expansions: [],
          narrowings: report.expansions,
          diff: { ...report.diff, added: 0, removed: 2, changed: 1 },
        },
      }),
    ).facts.items;
    expect(failing).toEqual([
      '没通过，2 个问题',
      '没有扩大项',
      `收窄 ${report.expansions.length} 项`,
      '改动 +0 −2 ~1',
      '同一对象还有 1 份更新的草案',
    ]);
  });
});

describe('队列里一行的文字：工作台待办', () => {
  it('标题去掉重复的类型前缀，灰字写项目，选中时多一行出处；动作是查看原文', () => {
    const entry = entries.find((candidate) => candidate.source === 'inbox')!;
    expect(queueText(entry, labels)).toEqual({
      title: '验收只读工作台',
      subtitle: '星舟',
      facts: { label: '出处', items: ['项目/星舟/00-总览/INDEX.md 第 3 行'] },
      action: {
        primary: '查看原文',
        link: '看原文',
        name: '验收只读工作台，查看原文',
      },
    });
    if (entry.source !== 'inbox') throw new Error('应是工作台待办');
    const prefixed: PendingEntry = {
      ...entry,
      action: { ...entry.action, title: '待验收：验收只读工作台' },
    };
    const text = queueText(prefixed);
    expect(text.title).toBe('验收只读工作台');
    // 读屏名字用完整原标题，与旧列表的按钮同名。
    expect(text.action.name).toBe('待验收：验收只读工作台，查看原文');
  });
});
