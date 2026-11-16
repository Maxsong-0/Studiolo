import { afterEach, describe, expect, it } from 'vitest';
import {
  GO_KEY,
  GO_TARGETS,
  GO_TIMEOUT_MS,
  QUEUE_KEYS,
  SHEET_KEY,
  SHORTCUT_GROUPS,
  createKeySequence,
  decideKey,
  isTypingTarget,
  normalizeKey,
  ownsEnter,
  type KeyLike,
  type KeyState,
} from '../../keys/shortcuts';

afterEach(() => {
  document.body.replaceChildren();
});

/**
 * 往页面里放一段标记，返回里面带 data-target 的那个元素（没有就取第一个）。
 * 标记全是本文件里写死的字面量，用解析器读成节点，不经 innerHTML。
 */
function target(html: string): Element {
  const parsed = new DOMParser().parseFromString(html, 'text/html');
  const holder = document.createElement('div');
  holder.append(...Array.from(parsed.body.childNodes));
  document.body.append(holder);
  return holder.querySelector('[data-target]') ?? holder.firstElementChild!;
}

function press(key: string, overrides: Partial<KeyLike> = {}): KeyLike {
  return {
    key,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    isComposing: false,
    repeat: false,
    defaultPrevented: false,
    target: document.body,
    ...overrides,
  };
}

/** 一份可拨的状态：时钟自己拨，登记了哪些键自己说。 */
function state(overrides: Partial<KeyState> = {}) {
  let now = 1_000_000;
  const sequence = createKeySequence(() => now);
  const bound = new Map<string, { repeat?: boolean; when?: boolean }>();
  const value: KeyState = {
    sequence,
    singleKeys: true,
    dialogOpen: false,
    sheetOpen: false,
    bound: (key, repeat) => {
      const binding = bound.get(key);
      return (
        binding !== undefined &&
        (!repeat || binding.repeat === true) &&
        binding.when !== false
      );
    },
    ...overrides,
  };
  return {
    value,
    sequence,
    bound,
    advance: (ms: number) => {
      now += ms;
    },
  };
}

describe('这个目标里能不能打字', () => {
  it('输入框、文本框、下拉框、可编辑区和打字用的角色都算', () => {
    for (const html of [
      '<input>',
      '<input type="text">',
      '<input type="search">',
      '<input type="number">',
      '<input type="password">',
      '<input type="date">',
      '<textarea></textarea>',
      '<select><option>一</option></select>',
      '<div contenteditable="true"></div>',
      '<div contenteditable=""></div>',
      '<div contenteditable="plaintext-only"></div>',
      '<div role="textbox"></div>',
      '<div role="combobox"></div>',
      '<div role="searchbox"></div>',
      '<div role="spinbutton"></div>',
      // 焦点在可编辑区里面的元素上也算。
      '<div contenteditable="true"><b data-target>粗体</b></div>',
    ])
      expect(isTypingTarget(target(html)), html).toBe(true);
  });

  it('按钮、链接、勾选框、单选、滑块、普通文字不算', () => {
    for (const html of [
      '<button>按钮</button>',
      '<a href="#x">链接</a>',
      '<input type="checkbox">',
      '<input type="radio">',
      '<input type="range">',
      '<input type="button" value="按钮">',
      '<input type="submit">',
      '<input type="file">',
      '<div contenteditable="false"></div>',
      '<p>文字</p>',
      '<div role="button"></div>',
    ])
      expect(isTypingTarget(target(html)), html).toBe(false);
    expect(isTypingTarget(document.body)).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
    expect(isTypingTarget(window)).toBe(false);
  });
});

describe('这个目标自己会不会处理 Enter', () => {
  it('按钮、链接、表单控件、折叠标题和按钮一类的角色会', () => {
    for (const html of [
      '<button>按钮</button>',
      '<a href="#x">链接</a>',
      '<input type="checkbox">',
      '<select><option>一</option></select>',
      '<textarea></textarea>',
      '<details><summary data-target>标题</summary></details>',
      '<div role="button"></div>',
      '<div role="link"></div>',
      '<div role="tab"></div>',
      '<div role="option"></div>',
      '<div role="menuitem"></div>',
      '<div contenteditable="true"></div>',
      // 按钮里面的图标、文字。
      '<button><span data-target>字</span></button>',
    ])
      expect(ownsEnter(target(html)), html).toBe(true);
  });

  it('没有 href 的 a、普通文字、页面本身不会', () => {
    for (const html of ['<a>没有去处</a>', '<p>文字</p>', '<li>一行</li>'])
      expect(ownsEnter(target(html)), html).toBe(false);
    expect(ownsEnter(document.body)).toBe(false);
    expect(ownsEnter(null)).toBe(false);
  });
});

describe('键名', () => {
  it('单个字符不分大小写，功能键原样', () => {
    expect(normalizeKey('J')).toBe('j');
    expect(normalizeKey('j')).toBe('j');
    expect(normalizeKey('?')).toBe('?');
    expect(normalizeKey('Enter')).toBe('Enter');
    expect(normalizeKey('Escape')).toBe('Escape');
  });
});

describe('G 之后等下一个键', () => {
  it('时限内算在等，过了就不算；等到一个键之后不再等', () => {
    let now = 0;
    const sequence = createKeySequence(() => now);
    expect(sequence.pending()).toBe(false);
    sequence.arm();
    expect(sequence.pending()).toBe(true);
    now = GO_TIMEOUT_MS;
    expect(sequence.pending()).toBe(true);
    now = GO_TIMEOUT_MS + 1;
    expect(sequence.pending()).toBe(false);
    sequence.arm();
    expect(sequence.resolve('p')).toBe('projects');
    expect(sequence.pending()).toBe(false);
    sequence.arm();
    expect(sequence.resolve('x')).toBeNull();
    expect(sequence.pending()).toBe(false);
    sequence.arm();
    sequence.cancel();
    expect(sequence.pending()).toBe(false);
  });

  it('六个字母各去一个视图，互不重复', () => {
    expect(GO_TARGETS.map(({ key, view }) => [key, view])).toEqual([
      ['h', 'cockpit'],
      ['p', 'projects'],
      ['d', 'documents'],
      ['c', 'collab'],
      ['r', 'rules'],
      ['s', 'policy'],
    ]);
    expect(new Set(GO_TARGETS.map(({ key }) => key)).size).toBe(6);
    expect(GO_TARGETS.map(({ key }) => key)).not.toContain(GO_KEY);
  });
});

describe('一次按键归谁', () => {
  it('G 再按字母：去那个视图；大写锁定开着也认', () => {
    const { value } = state();
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    expect(decideKey(press('p'), value)).toEqual({
      kind: 'go',
      view: 'projects',
    });
    expect(decideKey(press('G'), value)).toEqual({ kind: 'pending' });
    expect(decideKey(press('S'), value)).toEqual({
      kind: 'go',
      view: 'policy',
    });
    // 跳过之后不再等：下一个字母是普通按键。
    expect(decideKey(press('p'), value)).toEqual({ kind: 'none' });
  });

  it('G H 去驾驶舱：H 归这次跳转，不落到登记了 H 的地方', () => {
    const { value, bound } = state();
    bound.set('h', {});
    expect(decideKey(press('h'), value)).toEqual({ kind: 'bound', key: 'h' });
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    expect(decideKey(press('h'), value)).toEqual({
      kind: 'go',
      view: 'cockpit',
    });
  });

  it('G 之后按了表里没有的字符：这一键作废，也不落到登记了它的地方', () => {
    const { value, bound } = state();
    bound.set('j', {});
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    expect(decideKey(press('j'), value)).toEqual({ kind: 'cancelled' });
    expect(decideKey(press('j'), value)).toEqual({ kind: 'bound', key: 'j' });
    // 带 Shift 的字母不是跳转字母。
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    expect(decideKey(press('P', { shiftKey: true }), value)).toEqual({
      kind: 'cancelled',
    });
  });

  it('G 之后过了 1.5 秒：不算跳转，按键照常处理', () => {
    const { value, bound, advance } = state();
    bound.set('h', {});
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    advance(GO_TIMEOUT_MS + 1);
    expect(decideKey(press('h'), value)).toEqual({ kind: 'bound', key: 'h' });
  });

  it('G G：重新开始等', () => {
    const { value, advance } = state();
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    advance(GO_TIMEOUT_MS - 100);
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    advance(GO_TIMEOUT_MS - 100);
    expect(decideKey(press('d'), value)).toEqual({
      kind: 'go',
      view: 'documents',
    });
  });

  it('G 之后按功能键：序列作废，功能键照常处理；单按 Shift 不打断序列', () => {
    const { value, bound } = state();
    bound.set('Enter', {});
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    expect(decideKey(press('Enter'), value)).toEqual({
      kind: 'bound',
      key: 'Enter',
    });
    expect(decideKey(press('p'), value)).toEqual({ kind: 'none' });

    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    expect(decideKey(press('Shift', { shiftKey: true }), value)).toEqual({
      kind: 'none',
    });
    expect(decideKey(press('c'), value)).toEqual({
      kind: 'go',
      view: 'collab',
    });
  });

  it('「?」打开一览（带不带 Shift 都认）；别的带 Shift 的字母不算', () => {
    const { value, bound } = state();
    bound.set('j', {});
    expect(decideKey(press(SHEET_KEY, { shiftKey: true }), value)).toEqual({
      kind: 'sheet',
    });
    expect(decideKey(press(SHEET_KEY), value)).toEqual({ kind: 'sheet' });
    expect(decideKey(press('J', { shiftKey: true }), value)).toEqual({
      kind: 'none',
    });
    expect(decideKey(press('G', { shiftKey: true }), value)).toEqual({
      kind: 'none',
    });
  });

  it('按着 Ctrl、⌘、Alt，或输入法在组字，或别处已经处理掉：一律不管', () => {
    const { value, bound } = state();
    bound.set('j', {});
    for (const overrides of [
      { ctrlKey: true },
      { metaKey: true },
      { altKey: true },
      { isComposing: true },
      { defaultPrevented: true },
    ] satisfies Partial<KeyLike>[]) {
      expect(
        decideKey(press('j', overrides), value),
        JSON.stringify(overrides),
      ).toEqual({ kind: 'none' });
      expect(decideKey(press('g', overrides), value)).toEqual({ kind: 'none' });
      expect(decideKey(press('?', overrides), value)).toEqual({ kind: 'none' });
    }
  });

  it('焦点在能打字的地方：不管，在等的 G 也作废', () => {
    const { value, bound, sequence } = state();
    bound.set('j', {});
    const input = target('<input type="search">');
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    expect(decideKey(press('h', { target: input }), value)).toEqual({
      kind: 'none',
    });
    expect(sequence.pending()).toBe(false);
    for (const key of ['j', 'g', '?', 'Enter'])
      expect(decideKey(press(key, { target: input }), value), key).toEqual({
        kind: 'none',
      });
    expect(sequence.pending()).toBe(false);
  });

  it('有弹层开着：不管；开着的就是一览时，「?」还能把它关掉', () => {
    const { value: blocked, bound } = state({ dialogOpen: true });
    bound.set('j', {});
    for (const key of ['j', 'g', '?', 'Enter'])
      expect(decideKey(press(key), blocked), key).toEqual({ kind: 'none' });

    const { value: sheet, bound: sheetBound } = state({
      dialogOpen: true,
      sheetOpen: true,
    });
    sheetBound.set('j', {});
    expect(decideKey(press('?'), sheet)).toEqual({ kind: 'sheet' });
    expect(decideKey(press('?', { repeat: true }), sheet)).toEqual({
      kind: 'none',
    });
    for (const key of ['j', 'g', 'Enter'])
      expect(decideKey(press(key), sheet), key).toEqual({ kind: 'none' });
  });

  it('单键快捷键关掉了：J、G、「?」都不管', () => {
    const { value, bound } = state({ singleKeys: false });
    bound.set('j', {});
    for (const key of ['j', 'g', '?', 'h', 'Enter'])
      expect(decideKey(press(key), value), key).toEqual({ kind: 'none' });
  });

  it('Enter 落在按钮、链接上是它们自己的；落在别处才交给登记的地方', () => {
    const { value, bound } = state();
    bound.set('Enter', {});
    expect(decideKey(press('Enter'), value)).toEqual({
      kind: 'bound',
      key: 'Enter',
    });
    for (const html of ['<button>按钮</button>', '<a href="#x">链接</a>'])
      expect(
        decideKey(press('Enter', { target: target(html) }), value),
        html,
      ).toEqual({ kind: 'none' });
    // 字母键落在按钮上照样算：按钮不吃字母。
    bound.set('j', {});
    expect(
      decideKey(press('j', { target: target('<button>按钮</button>') }), value),
    ).toEqual({ kind: 'bound', key: 'j' });
  });

  it('驾驶舱的 A 批准、R 退回（T-200）：单独按交给登记的地方；「G R」是去规则页，不落到退回上', () => {
    const { value, bound, sequence } = state();
    bound.set(QUEUE_KEYS.approve, {});
    bound.set(QUEUE_KEYS.reject, {});
    expect(decideKey(press('a'), value)).toEqual({ kind: 'bound', key: 'a' });
    expect(decideKey(press('r'), value)).toEqual({ kind: 'bound', key: 'r' });
    // 开着大写锁定也认；带 Shift 的不算。
    expect(decideKey(press('A'), value)).toEqual({ kind: 'bound', key: 'a' });
    expect(decideKey(press('R', { shiftKey: true }), value)).toEqual({
      kind: 'none',
    });
    // 按住不放只认第一下：不会连着打开又收起。
    expect(decideKey(press('a', { repeat: true }), value)).toEqual({
      kind: 'none',
    });
    // G 之后的 R 归这次跳转。
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    expect(decideKey(press('r'), value)).toEqual({
      kind: 'go',
      view: 'rules',
    });
    expect(sequence.pending()).toBe(false);
    // G 之后的 A 不是跳转字母：这一键作废，也不落到批准上。
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    expect(decideKey(press('a'), value)).toEqual({ kind: 'cancelled' });
    // 打字的地方、弹层开着：都不算。
    expect(
      decideKey(press('a', { target: target('<input type="text" />') }), value),
    ).toEqual({ kind: 'none' });
    expect(decideKey(press('r'), { ...value, dialogOpen: true })).toEqual({
      kind: 'none',
    });
  });

  it('没有登记的键不管；登记方说焦点在这里不算数的也不管', () => {
    const { value, bound } = state();
    expect(decideKey(press('j'), value)).toEqual({ kind: 'none' });
    bound.set('Enter', { when: false });
    expect(decideKey(press('Enter'), value)).toEqual({ kind: 'none' });
  });

  it('按住不放：只有登记时说了可以连着触发的才连着来；G 和「?」只认第一下', () => {
    const { value, bound, sequence } = state();
    bound.set('j', { repeat: true });
    bound.set('h', {});
    expect(decideKey(press('j', { repeat: true }), value)).toEqual({
      kind: 'bound',
      key: 'j',
    });
    expect(decideKey(press('h', { repeat: true }), value)).toEqual({
      kind: 'none',
    });
    expect(decideKey(press('?', { repeat: true }), value)).toEqual({
      kind: 'none',
    });
    expect(decideKey(press('g', { repeat: true }), value)).toEqual({
      kind: 'none',
    });
    expect(sequence.pending()).toBe(false);
    // 在等 G 后面的键时，按住的 J 不算数，也不打断。
    expect(decideKey(press('g'), value)).toEqual({ kind: 'pending' });
    expect(decideKey(press('j', { repeat: true }), value)).toEqual({
      kind: 'none',
    });
    expect(sequence.pending()).toBe(true);
  });
});

describe('一览的内容和监听用的是同一张表', () => {
  const rows = SHORTCUT_GROUPS.flatMap((group) =>
    group.rows.map((row) => ({ group, row })),
  );
  const single = rows.filter(({ group }) => group.single);

  it('每个跳转字母都有一行「G 再按它」', () => {
    for (const { key, label } of GO_TARGETS)
      expect(
        single.some(
          ({ row }) =>
            row.does === `去${label}` &&
            row.ways.some(
              (way) =>
                way.sequence === true &&
                way.keys.join(' ') === `G ${key.toUpperCase()}`,
            ),
        ),
        label,
      ).toBe(true);
  });

  it('队列登记的每个键（J、K、Enter、H、A、R）和「?」都列着', () => {
    const listed = new Set(
      single.flatMap(({ row }) =>
        row.ways
          .filter((way) => !way.sequence)
          .flatMap((way) => way.keys.map(normalizeKey)),
      ),
    );
    for (const key of [...Object.values(QUEUE_KEYS), SHEET_KEY])
      expect(listed.has(normalizeKey(key)), key).toBe(true);
  });

  it('会写东西的两个键写明只到确认那一步', () => {
    const does = (key: string) =>
      single.find(({ row }) =>
        row.ways.some((way) => !way.sequence && way.keys.join('') === key),
      )?.row.does ?? '';
    expect(does('A')).toContain('只展开执行预览，点了确认才发布');
    expect(does('R')).toContain('只展开确认框，点了确认才撤回');
  });

  it('⌘K 那一组不算单键快捷键：关掉开关后它还在', () => {
    const palette = SHORTCUT_GROUPS.filter((group) => !group.single);
    expect(palette).toHaveLength(1);
    expect(
      palette[0].rows.some((row) =>
        row.ways.some((way) => way.keys.join('+') === '⌘+K'),
      ),
    ).toBe(true);
  });

  it('每一行的说明互不相同（一览按说明取键）', () => {
    for (const group of SHORTCUT_GROUPS) {
      const texts = group.rows.map((row) => row.does);
      expect(new Set(texts).size, group.title).toBe(texts.length);
    }
  });
});
