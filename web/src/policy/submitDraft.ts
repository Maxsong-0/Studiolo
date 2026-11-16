/**
 * 策略页「提交」里正在写的那一份草案（T-198）。只活在面板的内存里：换标签不丢，
 * 整页重挂就没了；不进位置、不进本机存储——正文绝不落到 URL 和浏览器里。
 */
import type { PolicyKind } from '../policyModel';

export interface SubmitDraft {
  kind: PolicyKind;
  /** 对象：规则文件名、skill 名或 AI 类型名；提交时去掉首尾空白。 */
  subject: string;
  body: string;
  /** 撤销该对象：勾上就不带正文。 */
  retire: boolean;
}

const CAPABILITY_TEMPLATE = JSON.stringify(
  {
    ai_type: 'claude',
    read: {
      project_whitelist: ['项目/Studiolo', '_规则'],
      deny_by_default: true,
      // 缺这一项＝一条规则都读不到（ADR-026 D4）；首批规则与 docs/m5 初始能力集一致。
      rules_whitelist: ['通用工作规则.md', 'CLAUDE.md'],
    },
    write: { board: false, content_patch_only: true, content_direct: false },
    policy: { propose_desired: true, activate: false },
    hard_bound_skills: [],
  },
  null,
  2,
);
const SKILL_TEMPLATE = JSON.stringify(
  {
    name: 'skill-name',
    summary: '一句摘要',
    triggers: [],
    markdown: '# 正文\n',
  },
  null,
  2,
);

/** 换类别时正文换成这一类的起手模板：能力集和 skill 是 JSON 骨架，规则是空的。 */
export function bodyTemplate(kind: PolicyKind): string {
  if (kind === 'capability_set') return CAPABILITY_TEMPLATE;
  return kind === 'skills' ? SKILL_TEMPLATE : '';
}

export const EMPTY_SUBMIT_DRAFT: SubmitDraft = {
  kind: 'capability_set',
  subject: '',
  body: bodyTemplate('capability_set'),
  retire: false,
};

/**
 * 生效版本的正文放进编辑区之前：能力集和 skill 重新排成多行（服务端存的是规范化后的 JSON，挤在一行里
 * 没法改）；规则是 Markdown，原样。排不了（不是 JSON）就原样放，不猜。
 */
export function editableBody(kind: PolicyKind, body: string): string {
  if (kind === 'rules') return body;
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return body;
  }
}

/**
 * 「从生效中的版本开始改」：把读回来的那一版正文放进正在写的草案，覆盖已经写的。
 * 正文是读回来的，其间人可能已经换了类别、换了对象，或者勾了「撤销该对象」——那就不是他要的那一份了，
 * 原样返回，不填。`target` 是点按钮那一刻的类别和对象。
 */
export function startFromBody(
  current: SubmitDraft,
  target: { kind: PolicyKind; subject: string },
  body: string,
): SubmitDraft {
  return current.kind === target.kind &&
    current.subject.trim() === target.subject &&
    !current.retire
    ? { ...current, body: editableBody(target.kind, body) }
    : current;
}
