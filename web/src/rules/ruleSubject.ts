/**
 * 规则名（subject）与正文的前端预校验（T-160，ADR-026 D1）：与后端 validate_rules_subject、
 * MAX_BODY_BYTES 同口径。只为早点告诉人哪里不对；最终裁决仍在服务端的校验报告里。
 */

/** 规则名最多几个字符；与后端 validate_rules_subject 同值。 */
export const RULE_SUBJECT_MAX_CHARS = 128;
/** 主人提交的母本正文上限（字节）；与后端 MAX_BODY_BYTES 同值。 */
export const RULE_BODY_MAX_BYTES = 256 * 1024;

const CONTROL = /\p{Cc}/u;
const ENCODED = /%(2f|5c|00|2e)/i;
/** 组合附加符号的码点区间（与 VaultPath 的 is_unicode_combining_mark 同一张表）。 */
const COMBINING: readonly (readonly [number, number])[] = [
  [0x0300, 0x036f],
  [0x1ab0, 0x1aff],
  [0x1dc0, 0x1dff],
  [0x20d0, 0x20ff],
  [0xfe20, 0xfe2f],
];

function hasCombiningMark(text: string): boolean {
  for (const character of text) {
    const code = character.codePointAt(0) ?? 0;
    if (COMBINING.some(([low, high]) => code >= low && code <= high))
      return true;
  }
  return false;
}

/**
 * 规则名有什么问题；合法返回 null。与后端 validate_rules_subject 同口径：
 * 以 .md 结尾、不超过 128 个字符、不含 /、不以 . 开头，并且是 VaultPath 接受的写法
 * （不含反斜杠、控制字符、百分号转义的分隔符与点、组合附加符号，须为 NFC）。
 */
export function ruleSubjectProblem(subject: string): string | null {
  if (!subject) return '请填写规则名。';
  if (!subject.endsWith('.md'))
    return '规则名须以 .md 结尾，例如「通用工作规则.md」。';
  if (subject.startsWith('.')) return '规则名不能以 . 开头。';
  if (subject.includes('/'))
    return '规则名不能含 /：规则文件直接放在 _规则/ 下，不分子目录。';
  if ([...subject].length > RULE_SUBJECT_MAX_CHARS)
    return `规则名最多 ${RULE_SUBJECT_MAX_CHARS} 个字符。`;
  if (
    subject.includes('\\') ||
    CONTROL.test(subject) ||
    ENCODED.test(subject) ||
    hasCombiningMark(subject) ||
    subject.normalize('NFC') !== subject
  )
    return '规则名不能含反斜杠、控制字符、百分号转义或组合附加符号，并须是 NFC 写法。';
  return null;
}

export function validRuleSubject(subject: string): boolean {
  return ruleSubjectProblem(subject) === null;
}

/** 正文的 UTF-8 字节数：上限按字节算，与后端一致。 */
export function bodyBytes(body: string): number {
  return new TextEncoder().encode(body).length;
}
