import { describe, expect, it } from 'vitest';
import {
  publishPreview,
  withdrawPreview,
  withdrawVerb,
} from '../../policy/previews';
import { desiredEnvelope, policyStatus } from '../../policyModel';
import type { DesiredRecord } from '../../policyModel';
import { DESIRED_ID, desiredBody, statusBody } from './policyFixtures';

type Body = Record<string, unknown>;

const record = (overrides: Body = {}): DesiredRecord =>
  desiredEnvelope({ desired: desiredBody(overrides) });

const PROPOSAL = record({
  status: 'pending',
  submitter_kind: 'agent',
  agent_type: 'claude',
  agent_instance: '01ABCDEFGHJKMNPQRSTVWXYZ77',
});
const OWN = record();

const fact = (
  content: ReturnType<typeof withdrawPreview>,
  label: string,
): string | undefined =>
  content.facts.find((item) => item.label === label)?.value;

describe('撤回一份草案在界面上叫什么', () => {
  it('AI 交的提案是「退回」，主人自己交的是「撤回」', () => {
    expect(withdrawVerb(PROPOSAL)).toBe('退回');
    expect(withdrawVerb(OWN)).toBe('撤回');
  });
});

describe('退回的确认框（T-200，驾驶舱）', () => {
  it('AI 的提案：写明对象、草案号（写全）、谁交的、结果', () => {
    const content = withdrawPreview(PROPOSAL);
    expect(content.title).toBe('退回 能力集 claude');
    expect(content.facts.map((item) => item.label)).toEqual([
      '对象',
      '草案',
      '提交者',
      '结果',
    ]);
    expect(fact(content, '对象')).toBe('能力集 claude');
    expect(fact(content, '草案')).toBe(DESIRED_ID);
    expect(fact(content, '提交者')).toBe('AI claude');
    expect(fact(content, '结果')).toBe(
      '这份草案标为已撤回，不再等你处理；生效中的内容不变',
    );
    expect(content.confirmLabel).toBe('确认退回');
  });

  it('自己交的草案：叫「撤回」，提交者是主人', () => {
    const content = withdrawPreview(OWN);
    expect(content.title).toBe('撤回 能力集 claude');
    expect(fact(content, '提交者')).toBe('主人');
    expect(content.confirmLabel).toBe('确认撤回');
  });

  it('照实说：撤回的草案不能恢复；这一步不要一次性验证码', () => {
    expect(withdrawPreview(PROPOSAL).notes).toEqual([
      '不可撤回：撤回的草案不能恢复；要再提，得重新提交一份。',
      '不需要一次性验证码：确认后立即提交。',
    ]);
  });

  it('没有红字：退回不是要逐条确认的扩大项', () => {
    expect(withdrawPreview(PROPOSAL).facts.some((item) => item.signal)).toBe(
      false,
    );
  });
});

describe('发布预览里随页面不同的两句话', () => {
  const status = policyStatus(statusBody());
  const report = OWN.report!;
  const expansions = (content: ReturnType<typeof publishPreview>) =>
    content.facts.find((item) => item.label === '扩大项');
  const DEFAULT_STEP_UP =
    '需要二次认证（一次性验证码）：服务端要求时会弹出窗口请你输入，通过后才会发布。';

  it('不给就是策略页、规则页的说法（原样不变）', () => {
    const content = publishPreview(OWN, report, status, [], []);
    expect(expansions(content)).toEqual({
      label: '扩大项',
      value: '1 项，见上方校验报告',
      signal: true,
    });
    expect(content.notes.at(-1)).toBe(DEFAULT_STEP_UP);
  });

  it('驾驶舱：扩大项写「上面已逐条列出」，认证那一句换成给的那一句', () => {
    const stepUp = '二次认证还有效约 4 分（以服务端为准）：确认后直接发布。';
    const content = publishPreview(OWN, report, status, [], [], {
      expansionsWhere: '上面已逐条列出',
      stepUp,
    });
    expect(expansions(content)?.value).toBe('1 项，上面已逐条列出');
    expect(content.notes.at(-1)).toBe(stepUp);
    expect(content.notes).not.toContain(DEFAULT_STEP_UP);
  });

  it('只换扩大项那一句：认证那一句还是缺省的', () => {
    const content = publishPreview(OWN, report, status, [], [], {
      expansionsWhere: '上面已逐条列出',
      stepUp: undefined,
    });
    expect(expansions(content)?.value).toBe('1 项，上面已逐条列出');
    expect(content.notes.at(-1)).toBe(DEFAULT_STEP_UP);
  });

  it('没有扩大项：写「无」，不带出处', () => {
    const none = record({
      report: { ...(desiredBody().report as Body), expansions: [] },
    });
    const content = publishPreview(none, none.report!, status, [], [], {
      expansionsWhere: '上面已逐条列出',
    });
    expect(expansions(content)).toEqual({
      label: '扩大项',
      value: '无',
      signal: false,
    });
  });

  it('其余事实和「不可撤回」那一句不随说法变', () => {
    const plain = publishPreview(OWN, report, status, [], []);
    const cockpit = publishPreview(OWN, report, status, [], [], {
      expansionsWhere: '上面已逐条列出',
      stepUp: '别的说法',
    });
    const others = (content: ReturnType<typeof publishPreview>) =>
      content.facts.filter((item) => item.label !== '扩大项');
    expect(others(cockpit)).toEqual(others(plain));
    expect(cockpit.title).toBe(plain.title);
    expect(cockpit.confirmLabel).toBe(plain.confirmLabel);
    expect(cockpit.notes.slice(0, -1)).toEqual(plain.notes.slice(0, -1));
  });
});
