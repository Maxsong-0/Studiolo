import { StrictMode } from 'react';
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';
import { PolicyBoundary } from '../../PolicyBoundary';
import { ApiError } from '../../sessionHttp';
import {
  DRAFT_CAPABILITY,
  capabilityJson,
  desiredBody,
  detailCell,
  openTab,
  policyServer,
  reportBody,
  statusBody,
  submitFields,
  tab,
  versionBody,
  type PolicyServer,
  type PolicyTabName,
} from './policyFixtures';

/**
 * 策略页的三个子标签（T-198）：在看哪个标签、选中哪份草案都写在位置里；各标签记住自己选中的那一项；
 * 提交之后把人带到「待处理」里刚交的那一份；「提交」里正在写的内容换标签不丢。
 */

const FIRST = '01ABCDEFGHJKMNPQRSTVWXYZ01';
const SECOND = '01ABCDEFGHJKMNPQRSTVWXYZ02';
const CREATED = '01ABCDEFGHJKMNPQRSTVWXYZ09';

/** 队列里的两份：一份能力集草案，一份规则草案。 */
const capabilityDraft = () => desiredBody({ id: FIRST });
const rulesDraft = () =>
  desiredBody({
    id: SECOND,
    kind: 'rules',
    subject: 'CLAUDE.md',
    body: '# 规则',
    report: reportBody({
      kind: 'rules',
      subject: 'CLAUDE.md',
      expansions: [],
      narrowings: [],
      current_version: null,
    }),
  });

let server: PolicyServer | undefined;
afterEach(() => {
  server?.uninstall();
  server = undefined;
});

/** 队列里某一份草案的那一行按钮。 */
const pick = (name: string) => screen.getByRole('button', { name });

/** 标签栏右边的等宽汇总。 */
function bar() {
  const element = screen
    .getByRole('tablist', { name: '策略面板' })
    .closest<HTMLElement>('.policy-bar');
  if (!element) throw new Error('缺少标签栏');
  return element;
}

/** 读过哪几版的正文，按先后。 */
const bodyReads = () =>
  server
    ?.reads()
    .filter((path) => path.startsWith('/api/v1/policy/versions/')) ?? [];

describe('三个子标签', () => {
  it('缺省是「现状」；标签栏有名字，面板由选中的标签命名；右边汇总写生效几项、几版；落地不改位置', async () => {
    server = policyServer({
      desired: [capabilityDraft()],
      versions: [versionBody(2), versionBody(1)],
    });
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });

    expect(
      within(screen.getByRole('tablist', { name: '策略面板' }))
        .getAllByRole('tab')
        .map((item) => [item.textContent, item.getAttribute('aria-selected')]),
    ).toEqual([
      ['现状', 'true'],
      ['待处理 1', 'false'],
      ['提交', 'false'],
    ]);
    const panel = screen.getByRole('tabpanel', { name: '现状' });
    expect(tab('现状')).toHaveAttribute('aria-controls', panel.id);
    expect(
      within(panel).getByRole('list', { name: '版本列表' }),
    ).toBeInTheDocument();
    expect(
      within(panel).getByRole('form', { name: '撤销接入' }),
    ).toBeInTheDocument();
    // 别的标签的内容不在这一屏。
    expect(
      screen.queryByRole('list', { name: '待处理草案' }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole('form', { name: '提交规则草案' }),
    ).not.toBeInTheDocument();

    expect(within(bar()).getByText('生效 1 项')).toBeInTheDocument();
    expect(within(bar()).getByText('2 版')).toBeInTheDocument();
    expect(within(bar()).queryByText('全局撤销中')).not.toBeInTheDocument();
    expect(window.location.hash).toBe('');
  });

  it('点标签换面板，位置跟着写：待处理 panel=pending、提交 panel=submit、现状不写；每个面板只有自己的内容', async () => {
    server = policyServer({
      desired: [capabilityDraft()],
      versions: [versionBody(2), versionBody(1)],
    });
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });

    await openTab(user, '待处理');
    expect(window.location.hash).toBe('#view=policy&panel=pending');
    const pending = screen.getByRole('tabpanel', { name: /^待处理/ });
    expect(
      within(pending).getByRole('list', { name: '待处理草案' }),
    ).toBeInTheDocument();
    expect(within(pending).getByRole('region', { name: '选中的草案' })).toBe(
      detailCell(),
    );
    expect(
      screen.queryByRole('table', { name: '生效中的各项' }),
    ).not.toBeInTheDocument();

    await openTab(user, '提交');
    expect(window.location.hash).toBe('#view=policy&panel=submit');
    const submit = screen.getByRole('tabpanel', { name: '提交' });
    expect(
      within(submit).getByRole('form', { name: '提交规则草案' }),
    ).toBeInTheDocument();
    expect(
      within(submit).getByRole('region', { name: '和生效中的差别' }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('list', { name: '待处理草案' }),
    ).not.toBeInTheDocument();

    await openTab(user, '现状');
    expect(window.location.hash).toBe('#view=policy');
    expect(
      screen.getByRole('table', { name: '生效中的各项' }),
    ).toBeInTheDocument();
  });

  const landings: [string, PolicyTabName][] = [
    ['#view=policy', '现状'],
    ['#view=policy&panel=pending', '待处理'],
    ['#view=policy&panel=submit', '提交'],
    // 不认识的值当现状，不猜。
    ['#view=policy&panel=queue', '现状'],
    ['#view=policy&panel=Pending', '现状'],
  ];
  it.each(landings)(
    '位置 %s 落在「%s」，位置原样不改（刷新、分享链接都回到同一个标签）',
    async (hash, name) => {
      server = policyServer({
        desired: [capabilityDraft()],
        versions: [versionBody(2), versionBody(1)],
      });
      window.location.hash = hash;
      render(<PolicyBoundary />);
      await screen.findByRole('tablist', { name: '策略面板' });
      expect(tab(name)).toHaveAttribute('aria-selected', 'true');
      expect(screen.getByRole('tabpanel')).toHaveAccessibleName(
        new RegExp(`^${name}`),
      );
      expect(window.location.hash).toBe(hash);
    },
  );

  it('左右方向键换标签，焦点跟着走，位置同样跟着写', async () => {
    server = policyServer({ desired: [capabilityDraft()] });
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });

    tab('现状').focus();
    await user.keyboard('{ArrowRight}');
    expect(tab('待处理')).toHaveAttribute('aria-selected', 'true');
    expect(tab('待处理')).toHaveFocus();
    expect(window.location.hash).toBe('#view=policy&panel=pending');
    await user.keyboard('{End}');
    expect(tab('提交')).toHaveFocus();
    expect(window.location.hash).toBe('#view=policy&panel=submit');
    // 到头了再往右回到第一个。
    await user.keyboard('{ArrowRight}');
    expect(tab('现状')).toHaveFocus();
    expect(window.location.hash).toBe('#view=policy');
  });
});

describe('待处理里选中哪一份', () => {
  it('没选时选第一份（只是展开，位置不写）；点另一份就换，位置记下草案号；重挂之后还是那一份', async () => {
    server = policyServer({
      desired: [capabilityDraft(), rulesDraft()],
      versions: [versionBody(2), versionBody(1)],
    });
    window.location.hash = '#view=policy&panel=pending';
    const user = userEvent.setup();
    const view = render(<PolicyBoundary />);
    await screen.findByRole('list', { name: '待处理草案' });

    expect(pick('能力集 claude')).toHaveAttribute('aria-current', 'true');
    expect(pick('规则母本 CLAUDE.md')).not.toHaveAttribute('aria-current');
    expect(
      within(detailCell()).getByRole('heading', { name: '能力集 claude' }),
    ).toBeInTheDocument();
    expect(window.location.hash).toBe('#view=policy&panel=pending');
    expect(server.posts).toEqual([]);

    await user.click(pick('规则母本 CLAUDE.md'));
    expect(window.location.hash).toBe(
      `#view=policy&panel=pending&item=${SECOND}`,
    );
    expect(pick('规则母本 CLAUDE.md')).toHaveAttribute('aria-current', 'true');
    expect(pick('能力集 claude')).not.toHaveAttribute('aria-current');
    expect(
      within(detailCell()).getByRole('heading', { name: '规则母本 CLAUDE.md' }),
    ).toBeInTheDocument();
    // 位置里只有草案号：正文不进 URL。
    expect(decodeURIComponent(window.location.hash)).not.toContain('# 规则');

    // 整页重挂（会话重新核对、刷新）：位置还在，回到同一份。
    view.unmount();
    render(<PolicyBoundary />);
    await screen.findByRole('list', { name: '待处理草案' });
    expect(pick('规则母本 CLAUDE.md')).toHaveAttribute('aria-current', 'true');
    expect(server.posts).toEqual([]);
  });

  it('带着草案号过来（驾驶舱的「去策略页处理」）：落地就选中那一份', async () => {
    server = policyServer({
      desired: [capabilityDraft(), rulesDraft()],
      versions: [versionBody(2)],
    });
    window.location.hash = `#view=policy&panel=pending&item=${SECOND}`;
    render(<PolicyBoundary />);
    await screen.findByRole('list', { name: '待处理草案' });
    expect(pick('规则母本 CLAUDE.md')).toHaveAttribute('aria-current', 'true');
    expect(
      within(detailCell()).getByRole('heading', { name: '规则母本 CLAUDE.md' }),
    ).toBeInTheDocument();
    // 选中的是规则草案：没有能力差异要比，一版正文都不读。
    expect(bodyReads()).toEqual([]);
  });

  it('位置里的草案号已经不在队列里（发布了、撤回了、别人处理了）：回到第一份，不报错，位置不改', async () => {
    server = policyServer({
      desired: [
        capabilityDraft(),
        rulesDraft(),
        desiredBody({
          id: CREATED,
          subject: 'codex',
          status: 'withdrawn',
        }),
      ],
      versions: [versionBody(2)],
    });
    window.location.hash = `#view=policy&panel=pending&item=${CREATED}`;
    render(<PolicyBoundary />);
    await screen.findByRole('list', { name: '待处理草案' });
    // 已撤回的那一份不在队列里。
    expect(tab('待处理')).toHaveTextContent('待处理 2');
    expect(
      screen.queryByRole('button', { name: '能力集 codex' }),
    ).not.toBeInTheDocument();
    expect(pick('能力集 claude')).toHaveAttribute('aria-current', 'true');
    expect(document.querySelector('.policy-feedback--error')).toBeNull();
    expect(window.location.hash).toBe(
      `#view=policy&panel=pending&item=${CREATED}`,
    );
  });

  it('队列空着：写明自己写的在「提交」里交、AI 提的会出现在这里', async () => {
    server = policyServer({ versions: [versionBody(2)] });
    window.location.hash = '#view=policy&panel=pending';
    render(<PolicyBoundary />);
    await screen.findByText('没有待处理的规则草案。');
    expect(tab('待处理')).toHaveTextContent('待处理 0');
    expect(
      within(detailCell()).getByText(
        '队列是空的。自己写的草案在「提交」里交；AI 提的会出现在这里。',
      ),
    ).toBeInTheDocument();
  });
});

describe('各标签记住自己选中的那一项', () => {
  it('现状里在读的版本、待处理里选中的草案，换走再回来都还在；位置里始终只有当前标签的那一项', async () => {
    server = policyServer({
      desired: [capabilityDraft(), rulesDraft()],
      versions: [versionBody(2), versionBody(1)],
    });
    window.location.hash = '#view=policy&item=1';
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    expect(await screen.findByText('v1 正文')).toBeInTheDocument();

    await openTab(user, '待处理');
    // 版本号是现状的，不带进待处理：没选过，就是第一份。
    expect(window.location.hash).toBe('#view=policy&panel=pending');
    expect(pick('能力集 claude')).toHaveAttribute('aria-current', 'true');
    await user.click(pick('规则母本 CLAUDE.md'));
    expect(window.location.hash).toBe(
      `#view=policy&panel=pending&item=${SECOND}`,
    );

    await openTab(user, '现状');
    expect(window.location.hash).toBe('#view=policy&item=1');
    expect(screen.getByText('v1 正文')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'v1' })).toHaveAttribute(
      'aria-current',
      'true',
    );

    await openTab(user, '待处理');
    expect(window.location.hash).toBe(
      `#view=policy&panel=pending&item=${SECOND}`,
    );
    expect(pick('规则母本 CLAUDE.md')).toHaveAttribute('aria-current', 'true');

    // 提交没有「选中的那一项」；经过它再回来，两边记着的都还在。
    await openTab(user, '提交');
    expect(window.location.hash).toBe('#view=policy&panel=submit');
    await openTab(user, '待处理');
    expect(pick('规则母本 CLAUDE.md')).toHaveAttribute('aria-current', 'true');
    await openTab(user, '现状');
    expect(screen.getByText('v1 正文')).toBeInTheDocument();

    // v1 的正文只读了一次（回来时用记着的那一份）；v2 是能力差异的对比基准，也只读一次。
    expect([...bodyReads()].sort()).toEqual([
      '/api/v1/policy/versions/1',
      '/api/v1/policy/versions/2',
    ]);
  });

  it('旧写法 item=desired 带路过来的不算选中的草案：换走再回来，不把它当草案号也不当版本号', async () => {
    server = policyServer({
      desired: [capabilityDraft(), rulesDraft()],
      versions: [versionBody(2), versionBody(1)],
    });
    window.location.hash = '#view=policy&item=desired';
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('list', { name: '待处理草案' });

    await openTab(user, '现状');
    expect(window.location.hash).toBe('#view=policy');
    await openTab(user, '待处理');
    expect(window.location.hash).toBe('#view=policy&panel=pending');
    expect(pick('能力集 claude')).toHaveAttribute('aria-current', 'true');
    expect(bodyReads()).toEqual(['/api/v1/policy/versions/2']);
  });
});

describe('提交之后', () => {
  /** 提交接口：交上去的那一份进列表（与真实接口一样，重读的列表里就有它）。 */
  function submitting(reply: (input: Record<string, unknown>) => unknown) {
    const listed = [capabilityDraft()];
    server = policyServer({
      desired: listed,
      versions: [versionBody(2)],
      bodies: { 2: capabilityJson() },
      write: (path, body) => {
        if (path !== '/api/v1/policy/desired') throw new ApiError('未知', 404);
        const created = reply(body as Record<string, unknown>) as Record<
          string,
          unknown
        >;
        listed.push(created);
        return { desired: created };
      },
    });
    return server;
  }

  /** 打开「提交」，填上对象和一份能力集正文。 */
  async function fill(user: ReturnType<typeof userEvent.setup>) {
    await openTab(user, '提交');
    await user.type(submitFields().subject, 'codex');
    fireEvent.change(submitFields().body, {
      target: { value: DRAFT_CAPABILITY },
    });
  }

  it('交成功：切到「待处理」并选中刚交的那一份，结果那一句落在那一格；回到「提交」，写的内容还在', async () => {
    const current = submitting((input) =>
      desiredBody({
        id: CREATED,
        subject: input.subject,
        body: input.body,
        report: reportBody({ subject: input.subject, current_version: null }),
      }),
    );
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });
    await fill(user);
    await user.click(submitFields().submit);

    // 点提交那一刻人还在「提交」里：等被带到「待处理」，结果那一句在选中的草案那一格。
    const landed = await screen.findByRole('region', { name: '选中的草案' });
    expect(
      within(landed).getByText(
        `已提交并校验通过（${CREATED}）；发布前请逐条确认扩大项。`,
      ),
    ).toHaveAttribute('role', 'status');
    expect(current.posts).toEqual([
      {
        path: '/api/v1/policy/desired',
        body: {
          kind: 'capability_set',
          subject: 'codex',
          body: DRAFT_CAPABILITY,
        },
      },
    ]);
    expect(tab('待处理')).toHaveAttribute('aria-selected', 'true');
    expect(tab('待处理')).toHaveTextContent('待处理 2');
    expect(window.location.hash).toBe(
      `#view=policy&panel=pending&item=${CREATED}`,
    );
    expect(pick('能力集 codex')).toHaveAttribute('aria-current', 'true');
    expect(
      within(detailCell()).getByRole('heading', { name: '能力集 codex' }),
    ).toBeInTheDocument();

    await openTab(user, '提交');
    expect(submitFields().subject).toHaveValue('codex');
    expect(submitFields().body).toHaveValue(DRAFT_CAPABILITY);
    // 再回「待处理」：还是刚交的那一份。
    await openTab(user, '待处理');
    expect(pick('能力集 codex')).toHaveAttribute('aria-current', 'true');
  });

  it('交了但校验没过：照样带到「待处理」里那一份，说明没过，校验报告里列着问题', async () => {
    submitting((input) =>
      desiredBody({
        id: CREATED,
        subject: input.subject,
        body: input.body,
        status: 'rejected',
        report: reportBody({
          ok: false,
          subject: input.subject,
          problems: [{ path: 'policy.activate', message: '永久为 false' }],
          expansions: [],
          narrowings: [],
          current_version: null,
        }),
      }),
    );
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });
    await fill(user);
    await user.click(submitFields().submit);

    const landed = await screen.findByRole('region', { name: '选中的草案' });
    expect(
      within(landed).getByText(
        `已提交但校验未通过（${CREATED}）；请按逐条问题修改后重新提交。`,
      ),
    ).toHaveAttribute('role', 'status');
    expect(pick('能力集 codex')).toHaveAttribute('aria-current', 'true');
    expect(within(detailCell()).getByText('永久为 false')).toBeInTheDocument();
    expect(
      within(detailCell()).getByRole('button', { name: '发布（需二次认证）' }),
    ).toBeDisabled();
  });

  it('没交成：留在「提交」，原因写在表单那一格，写的内容不丢，位置不动', async () => {
    server = policyServer({
      desired: [capabilityDraft()],
      write: () => {
        throw new ApiError('对象名不合法。', 400);
      },
    });
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });
    await fill(user);
    await user.click(submitFields().submit);

    const failure = await screen.findByText('对象名不合法。');
    expect(failure).toHaveAttribute('role', 'alert');
    expect(
      screen.getByRole('region', { name: '提交规则草案 desired' }),
    ).toContainElement(failure);
    expect(tab('提交')).toHaveAttribute('aria-selected', 'true');
    expect(window.location.hash).toBe('#view=policy&panel=submit');
    expect(submitFields().subject).toHaveValue('codex');
    expect(submitFields().body).toHaveValue(DRAFT_CAPABILITY);
    await waitFor(() => expect(submitFields().submit).toBeEnabled());
  });

  it('正在写的草案换标签不丢：类别、对象、正文、「撤销该对象」都还在', async () => {
    server = policyServer({ desired: [capabilityDraft()] });
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });
    await openTab(user, '提交');
    await user.selectOptions(submitFields().kind, 'rules');
    // 换类别就换成这一类的起手模板：规则是空的。
    expect(submitFields().body).toHaveValue('');
    await user.type(submitFields().subject, 'CLAUDE.md');
    await user.type(submitFields().body, '# 新规则');
    await user.click(submitFields().retire);

    await openTab(user, '现状');
    await openTab(user, '待处理');
    await openTab(user, '提交');
    expect(submitFields().kind).toHaveValue('rules');
    expect(submitFields().subject).toHaveValue('CLAUDE.md');
    expect(submitFields().body).toHaveValue('# 新规则');
    expect(submitFields().retire).toBeChecked();
    // 正文只在内存里：不进位置，也没有发出任何请求。
    expect(decodeURIComponent(window.location.hash)).toBe(
      '#view=policy&panel=submit',
    );
    expect(server.posts).toEqual([]);
  });
});

describe('全局撤销', () => {
  it('生效时三个标签上都写着「全局撤销中」；解除只在「现状」里', async () => {
    server = policyServer({
      status: statusBody({ global_revoked: true }),
      desired: [capabilityDraft()],
    });
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });

    const release = '解除全局撤销（需二次认证）';
    expect(within(bar()).getByText('全局撤销中')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: release })).toBeInTheDocument();
    for (const name of ['待处理', '提交'] as const) {
      await openTab(user, name);
      expect(within(bar()).getByText('全局撤销中')).toBeInTheDocument();
      expect(
        screen.queryByRole('button', { name: release }),
      ).not.toBeInTheDocument();
    }
  });
});

describe('StrictMode（开发模式下组件装载两次）', () => {
  it('照样读得到：现状里读回位置里的那一版正文，待处理里列出能力差异', async () => {
    server = policyServer({
      desired: [desiredBody({ id: FIRST, body: DRAFT_CAPABILITY })],
      versions: [versionBody(2), versionBody(1)],
      bodies: { 2: capabilityJson() },
    });
    window.location.hash = '#view=policy&item=1';
    const user = userEvent.setup();
    render(
      <StrictMode>
        <PolicyBoundary />
      </StrictMode>,
    );
    expect(await screen.findByText('v1 正文')).toBeInTheDocument();

    await openTab(user, '待处理');
    expect(
      await within(detailCell()).findByRole('table', {
        name: '能力集的每一项：现在和改后',
      }),
    ).toBeInTheDocument();
    await openTab(user, '现状');
    expect(screen.getByText('v1 正文')).toBeInTheDocument();
  });
});
