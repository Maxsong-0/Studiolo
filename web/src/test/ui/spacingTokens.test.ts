import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * 字号、间距、控件高度守卫（设计规范第 1～3 节；T-191）。jsdom 不加载样式表，只能核对样式表源文本。
 * web/src 下除 tokens.css 以外的每个样式文件：
 *  1. font-size 不许写 px 字面量，只引用 --type-*；
 *  2. padding／margin／gap 的独立分量不许写 px 字面量（0、1px、-1px 是细线补偿，除外）；
 *     函数里的 px 只许出现在混了别的单位的算式里（calc((1.6em - 9px) / 2) 这类）；
 *  3. min-height／height／min-width／width 等于某档控件高度时必须引用 --control-*。
 * 和别的尺寸咬合、挪 1px 就对不齐的几何值逐条登记在 GEOMETRY；登记了却用不上的也算失败。
 */

const SRC = join(import.meta.dirname, '../..');
const TOKENS = 'styles/tokens.css';

type Finding = {
  file: string;
  line: number;
  selector: string;
  property: string;
  value: string;
  why: string;
};

type Exemption = {
  file: string;
  selector: string;
  property: string;
  value: string;
  why: string;
};

/** 几何值：原值保留。改这张表之前先确认它真的和别的尺寸咬合，而不是图省事。 */
const GEOMETRY: Exemption[] = [
  {
    file: 'styles/tree.css',
    selector: '.tree-children',
    property: 'margin',
    value: '13px',
    why: '下级目录的竖线要落在展开箭头的中线下方',
  },
  {
    file: 'styles/tree.css',
    selector: '.tree-children',
    property: 'margin-left',
    value: '9px',
    why: '窄屏同上：箭头靠左了，竖线跟着挪',
  },
  {
    file: 'styles/architecture.css',
    selector: '.architecture-steps > li + li',
    property: 'padding-top',
    value: '26px',
    why: '步骤之间的连线 19px 加箭头 7px',
  },
  {
    file: 'styles/architecture.css',
    selector: '.architecture-steps button',
    property: 'padding',
    value: '11px',
    why: '左内边距和上方连线的位置（left: 22px）咬合',
  },
  {
    file: 'styles/collab.css',
    selector: '.collab-select__control',
    property: 'padding',
    value: '30px',
    why: '右侧给下拉箭头记号留的位置',
  },
  {
    file: 'styles/policy.css',
    selector: '.policy-select select',
    property: 'padding-right',
    value: '34px',
    why: '右侧给下拉箭头记号留的位置',
  },
  {
    file: 'styles/projects.css',
    selector: '.project-field--select select',
    property: 'padding',
    value: '36px',
    why: '右侧给下拉箭头记号留的位置',
  },
  {
    file: 'styles/file-operations.css',
    selector: '.file-op__confirm input',
    property: 'margin',
    value: '3px',
    why: '16px 的复选框和首行文字居中对齐',
  },
  {
    file: 'styles/file-icons.css',
    selector: '.file-type-icon.file-type-badge',
    property: 'width',
    value: '36px',
    why: '文件类型徽标是图标框，不是控件',
  },
  {
    file: 'styles/file-icons.css',
    selector: '.file-type-icon.file-type-badge',
    property: 'height',
    value: '36px',
    why: '文件类型徽标是图标框，不是控件',
  },
];

const SPACING =
  /^(?:(?:padding|margin)(?:-(?:top|right|bottom|left|inline|block|inline-start|inline-end|block-start|block-end))?|gap|row-gap|column-gap)$/;
const SIZE = /^(?:min-height|height|min-width|width)$/;
const PX = /-?\d+(?:\.\d+)?px/;
const PX_ONLY = /^-?\d+(?:\.\d+)?px$/;
const HAIRLINE = new Set(['0px', '1px', '-1px']);
/** 算式里除了 px 还有别的量：相对单位、环境变量或令牌。 */
const MIXED = /\d(?:em|rem|%|vh|dvh|svh|lvh|vw|cqi|ch)\b|env\(|var\(/;

/** 注释换成等长空白：行号不变，注释里的声明不算数。 */
function withoutComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, (comment) =>
    comment.replace(/[^\n]/g, ' '),
  );
}

/** 按顶层空白切分量，括号里的不切。 */
function topLevelParts(value: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let current = '';
  for (const char of value) {
    if (char === '(') depth += 1;
    if (char === ')') depth -= 1;
    if (/\s/.test(char) && depth === 0) {
      if (current) parts.push(current);
      current = '';
    } else current += char;
  }
  if (current) parts.push(current);
  return parts;
}

/** 声明所在规则的选择器：往回找最近的 {，取它前面那一段，空白压成一个空格。 */
function selectorAt(source: string, offset: number): string {
  const brace = source.lastIndexOf('{', offset);
  if (brace < 0) return '';
  const begin =
    Math.max(
      source.lastIndexOf('}', brace - 1),
      source.lastIndexOf(';', brace - 1),
      source.lastIndexOf('{', brace - 1),
    ) + 1;
  return source.slice(begin, brace).trim().replace(/\s+/g, ' ');
}

/** 一个样式文件里违反三条规则的声明；controls 是各档控件高度的 px 值。 */
function findings(
  file: string,
  css: string,
  controls: ReadonlySet<string>,
): Finding[] {
  const source = withoutComments(css);
  const declaration =
    /(?<![\w-])([a-z][a-z-]*)\s*:\s*([^;{}]+?)\s*(?:!important\s*)?;/g;
  const found: Finding[] = [];
  for (const match of source.matchAll(declaration)) {
    // @media (min-width: 760px) 里的不是声明：它前面紧挨着左括号
    if (source.slice(0, match.index).trimEnd().endsWith('(')) continue;
    const [, property, value] = match;
    const at = {
      file,
      line: source.slice(0, match.index).split('\n').length,
      selector: selectorAt(source, match.index),
      property,
    };
    if (property === 'font-size' && PX.test(value))
      found.push({ ...at, value, why: '字号写死了 px，改引用 --type-*' });
    if (SPACING.test(property))
      for (const part of topLevelParts(value)) {
        if (PX_ONLY.test(part)) {
          if (!HAIRLINE.has(part))
            found.push({
              ...at,
              value: part,
              why: '间距写死了 px，改引用 --space-*',
            });
        } else if (PX.test(part) && !MIXED.test(part))
          found.push({
            ...at,
            value: part,
            why: '算式里只有 px，改引用 --space-*',
          });
      }
    if (SIZE.test(property) && controls.has(value))
      found.push({ ...at, value, why: '控件高度写死了，改引用 --control-*' });
  }
  return found;
}

function exempted(finding: Finding, exemption: Exemption): boolean {
  return (
    finding.file === exemption.file &&
    finding.selector === exemption.selector &&
    finding.property === exemption.property &&
    finding.value === exemption.value
  );
}

function stylesheets(): string[] {
  return readdirSync(SRC, { encoding: 'utf8', recursive: true })
    .filter((path) => path.endsWith('.css') && path !== TOKENS)
    .sort();
}

/** tokens.css 里 :root 第一块定义的字号、间距、控件高度令牌：名字 → 值。 */
function scaleTokens(): Map<string, string> {
  const source = withoutComments(readFileSync(join(SRC, TOKENS), 'utf8'));
  const open = source.indexOf('{', source.indexOf(':root'));
  const body = source.slice(open + 1, source.indexOf('}', open));
  const pattern = /(--(?:type|space|control)-[\w-]+)\s*:\s*([^;]+?)\s*;/g;
  return new Map(
    Array.from(body.matchAll(pattern), ([, name, value]) => [name, value]),
  );
}

function controlHeights(): Set<string> {
  const heights = new Set<string>();
  for (const [name, value] of scaleTokens())
    if (name.startsWith('--control-')) heights.add(value);
  return heights;
}

describe('规则本身：故意写死的值会被拦下', () => {
  const controls = new Set(['44px', '40px', '36px', '32px']);
  const check = (css: string) =>
    findings('x.css', css, controls).map(
      ({ property, value }) => `${property}: ${value}`,
    );

  it('字号写 px 被拦；引用令牌、相对单位、取小值的算式不拦', () => {
    expect(check('a { font-size: 13px; }')).toEqual(['font-size: 13px']);
    expect(
      check(
        'a { font-size: var(--type-small); } b { font-size: 0.9em; } c { font-size: min(var(--type-giant), 80cqi); }',
      ),
    ).toEqual([]);
  });

  it('间距的独立分量写 px 被拦，每个分量各算一条', () => {
    expect(check('a { padding: 7px var(--space-3) 9px; }')).toEqual([
      'padding: 7px',
      'padding: 9px',
    ]);
    expect(check('a { margin-top: -6px; gap: 5px 22px; }')).toEqual([
      'margin-top: -6px',
      'gap: 5px',
      'gap: 22px',
    ]);
  });

  it('0、1px、-1px、百分比、auto 和混了别的单位的算式不拦', () => {
    expect(
      check(
        'a { margin: -1px auto 0 1px; padding: 0 5%; gap: calc((1.6em - 9px) / 2); padding-bottom: calc(47px + env(safe-area-inset-bottom) + 24px); margin-left: calc(-1 * var(--space-4)); }',
      ),
    ).toEqual([]);
  });

  it('算式里只有 px 也算写死', () => {
    expect(check('a { margin-top: calc(7px + 2px); }')).toEqual([
      'margin-top: calc(7px + 2px)',
    ]);
  });

  it('等于某档控件高度的尺寸必须引用令牌；别的尺寸不管', () => {
    expect(check('a { min-height: 44px; width: 32px; }')).toEqual([
      'min-height: 44px',
      'width: 32px',
    ]);
    expect(
      check('a { min-height: var(--control-touch); height: 46px; }'),
    ).toEqual([]);
  });

  it('注释里的声明、媒体查询的条件、自定义属性的定义不算', () => {
    expect(
      check(
        '/* a { padding: 7px; } */ @media (min-width: 760px) { a { --gutter: 7px; scroll-padding: 7px; line-height: 20px; } }',
      ),
    ).toEqual([]);
  });

  it('报出文件、行号和所在规则', () => {
    const [finding] = findings(
      'styles/demo.css',
      '.a {\n  color: red;\n}\n@media (width < 760px) {\n  .b > .c {\n    gap: 7px;\n  }\n}\n',
      controls,
    );
    expect(finding).toMatchObject({
      file: 'styles/demo.css',
      line: 6,
      selector: '.b > .c',
      property: 'gap',
      value: '7px',
    });
  });
});

describe('web/src 的样式文件', () => {
  const controls = controlHeights();
  const all = stylesheets().flatMap((file) =>
    findings(file, readFileSync(join(SRC, file), 'utf8'), controls),
  );

  it('扫到了样式文件，四档控件高度读得出来', () => {
    expect(stylesheets().length).toBeGreaterThan(30);
    expect([...controls].sort()).toEqual(['32px', '36px', '40px', '44px']);
  });

  it('没有登记之外的写死值', () => {
    const unexpected = all
      .filter((finding) => !GEOMETRY.some((entry) => exempted(finding, entry)))
      .map(
        ({ file, line, selector, property, value, why }) =>
          `${file}:${line} ${selector} { ${property}: ${value} }：${why}`,
      );
    expect(unexpected).toEqual([]);
  });

  it('登记的几何值都还在：删掉或改掉了就把登记一起去掉', () => {
    const stale = GEOMETRY.filter(
      (entry) => !all.some((finding) => exempted(finding, entry)),
    ).map(({ file, selector, property, value }) => {
      return `${file} ${selector} { ${property}: ${value} }`;
    });
    expect(stale).toEqual([]);
  });

  it('引用的字号、间距、控件高度令牌都在 tokens.css 里定义过', () => {
    const defined = scaleTokens();
    const reference = /var\((--(?:type|space|control)-[\w-]+)\)/g;
    const unknown = new Set<string>();
    for (const file of stylesheets()) {
      const source = withoutComments(readFileSync(join(SRC, file), 'utf8'));
      for (const [, name] of source.matchAll(reference))
        if (!defined.has(name)) unknown.add(`${file}：${name}`);
    }
    expect([...unknown]).toEqual([]);
  });

  it('间距主档是 4 的倍数，半档只有登记过的五个值', () => {
    const halves: string[] = [];
    for (const [name, value] of scaleTokens()) {
      const step = /^--space-(\d+)(-5)?$/.exec(name);
      if (!step) continue;
      const expected = (Number(step[1]) + (step[2] ? 0.5 : 0)) * 4;
      expect(value, name).toBe(`${expected}px`);
      if (step[2]) halves.push(value);
    }
    expect(halves).toEqual(['2px', '6px', '10px', '14px', '18px']);
  });
});
