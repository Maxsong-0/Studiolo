import { describe, expect, it } from 'vitest';
import { compareCapability } from '../../policy/capabilityDiff';
import {
  EMPTY_SUBMIT_DRAFT,
  bodyTemplate,
  editableBody,
  startFromBody,
  type SubmitDraft,
} from '../../policy/submitDraft';

describe('「提交」里正在写的草案（T-198）', () => {
  it('起手是一份能力集模板：差异表读得出它的结构；换类别换成那一类的模板，规则是空的', () => {
    expect(EMPTY_SUBMIT_DRAFT).toEqual({
      kind: 'capability_set',
      subject: '',
      body: bodyTemplate('capability_set'),
      retire: false,
    });
    expect(
      compareCapability(null, bodyTemplate('capability_set')),
    ).toMatchObject({ kind: 'rows' });
    expect(Object.keys(JSON.parse(bodyTemplate('skills')) as object)).toEqual([
      'name',
      'summary',
      'triggers',
      'markdown',
    ]);
    expect(bodyTemplate('rules')).toBe('');
  });

  it('放进编辑区之前：能力集和 skill 排成多行，规则原样；排不了（不是 JSON）就原样放', () => {
    expect(
      editableBody('capability_set', '{"ai_type":"claude","read":{}}'),
    ).toBe('{\n  "ai_type": "claude",\n  "read": {}\n}');
    expect(editableBody('skills', '{"name":"x"}')).toBe('{\n  "name": "x"\n}');
    expect(editableBody('rules', '{"看着像 JSON":1}')).toBe(
      '{"看着像 JSON":1}',
    );
    expect(editableBody('rules', '# 规则\n\n- 一条')).toBe('# 规则\n\n- 一条');
    expect(editableBody('capability_set', '不是 JSON')).toBe('不是 JSON');
  });
});

describe('从生效中的版本开始改', () => {
  const writing: SubmitDraft = {
    kind: 'capability_set',
    subject: ' claude ',
    body: '写了一半',
    retire: false,
  };
  const target = { kind: 'capability_set', subject: 'claude' } as const;

  it('类别、对象都没换：读回来的正文排好放进去，覆盖已经写的；别的不动', () => {
    expect(startFromBody(writing, target, '{"ai_type":"claude"}')).toEqual({
      kind: 'capability_set',
      subject: ' claude ',
      body: '{\n  "ai_type": "claude"\n}',
      retire: false,
    });
  });

  it('读回来的时候人已经换了对象、换了类别，或者勾了「撤销该对象」：不是他要的那一份了，原样不动', () => {
    for (const changed of [
      { ...writing, subject: 'codex' },
      { ...writing, kind: 'rules' as const },
      { ...writing, retire: true },
    ])
      expect(startFromBody(changed, target, '{"ai_type":"claude"}')).toBe(
        changed,
      );
  });

  it('规则正文是 Markdown：原样放进去', () => {
    const rules: SubmitDraft = {
      kind: 'rules',
      subject: 'CLAUDE.md',
      body: '',
      retire: false,
    };
    expect(
      startFromBody(
        rules,
        { kind: 'rules', subject: 'CLAUDE.md' },
        '# 规则\n\n- 一条',
      ).body,
    ).toBe('# 规则\n\n- 一条');
  });
});
