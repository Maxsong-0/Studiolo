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
import { jsonResponse } from '../fixtures';
import {
  DRAFT_CAPABILITY,
  HASH,
  capabilityJson,
  desiredBody,
  detailCell,
  openTab,
  policyServer,
  reportBody,
  statusBody,
  submitFields,
  versionBody,
  type PolicyServer,
} from './policyFixtures';

/**
 * 能力差异表接在页面上（T-198）：「待处理」里排在校验报告前面，基准是报告写的那一版；
 * 「提交」里随输入更新。基准正文读不到、对不上只影响这张表，说明原因；报告、发布、提交照常。
 */

const TABLE = '能力集的每一项：现在和改后';

let server: PolicyServer | undefined;
afterEach(() => {
  server?.uninstall();
  server = undefined;
});

/** 读过哪几版的正文，按先后。 */
const bodyReads = () =>
  server
    ?.reads()
    .filter((path) => path.startsWith('/api/v1/policy/versions/')) ?? [];

/** 让某一版正文的读取先失败；调用返回的函数之后恢复正常。 */
function failBody(current: PolicyServer, version: number) {
  const respond = current.fetchMock.getMockImplementation();
  if (!respond) throw new Error('fetch 还没装');
  let failing = true;
  current.fetchMock.mockImplementation(async (input, init) =>
    failing && String(input) === `/api/v1/policy/versions/${version}`
      ? jsonResponse({ error: '读取不可用' }, 503)
      : respond(input, init),
  );
  return () => {
    failing = false;
  };
}

const headers = (table: HTMLElement) =>
  within(table)
    .getAllByRole('columnheader')
    .map((header) => header.textContent);

/** 名单里某一项所在的那一条。 */
function entry(scope: HTMLElement, text: string) {
  const item = within(scope).getByText(text).closest('li');
  if (!item) throw new Error(`缺少名单项 ${text}`);
  return item;
}

const entryOf = (kind: string, subject: string, version: number) => ({
  kind,
  subject,
  version,
  body_sha256: HASH,
  activated_at: '2026-09-17T00:00:00Z',
});

describe('待处理：选中的能力集草案改了哪几项能力', () => {
  /** 直接落在「待处理」，等选中的那一份展开；返回「能力差异」那一块。 */
  async function openDraft() {
    window.location.hash = '#view=policy&panel=pending';
    render(<PolicyBoundary />);
    await screen.findByRole('list', { name: '待处理草案' });
    return within(detailCell()).getByRole('group', { name: '能力差异' });
  }

  it('校验报告前面有一张「现在／改后」表：基准是报告写的那一版，扩大、收窄照报告标在对应的项上', async () => {
    server = policyServer({
      desired: [desiredBody({ body: DRAFT_CAPABILITY })],
      versions: [versionBody(2), versionBody(1)],
      bodies: { 2: capabilityJson() },
    });
    const group = await openDraft();
    const table = await within(group).findByRole('table', { name: TABLE });
    expect(headers(table)).toEqual(['能力', '现在生效中 v2', '改后这份草案']);
    expect(
      within(group).getByText('1 项有改动，10 项不变'),
    ).toBeInTheDocument();
    // 先只列有改动的那一行（外加表头）；不变的十项收在开关后面。
    expect(within(table).getAllByRole('row')).toHaveLength(2);
    expect(
      within(group).getByRole('button', { name: '列出不变的 10 项' }),
    ).toHaveAttribute('aria-expanded', 'false');

    const added = entry(table, '项目/Semiment');
    expect(within(added).getByText('新增')).toBeInTheDocument();
    expect(within(added).getByText('扩大')).toBeInTheDocument();
    const removed = entry(table, '项目/旧项目');
    expect(within(removed).getByText('去掉')).toBeInTheDocument();
    expect(within(removed).getByText('收窄')).toBeInTheDocument();

    // 排在校验报告前面。
    const report = within(detailCell()).getByRole('group', {
      name: '校验报告',
    });
    expect(
      group.compareDocumentPosition(report) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    // 只读了基准那一版，一次。
    expect(bodyReads()).toEqual(['/api/v1/policy/versions/2']);
    expect(server.posts).toEqual([]);
  });

  it('基准正文没读到：只是这张表列不出来，报告和发布照常、不报错；「再读一次」读到了就列出来', async () => {
    server = policyServer({
      desired: [desiredBody({ body: DRAFT_CAPABILITY })],
      versions: [versionBody(2)],
      bodies: { 2: capabilityJson() },
    });
    const recover = failBody(server, 2);
    const user = userEvent.setup();
    const group = await openDraft();

    expect(
      await within(group).findByText(
        '没读到 v2 的正文，这张表列不出来；校验报告和发布不受影响。',
      ),
    ).toBeInTheDocument();
    expect(within(group).queryByRole('table')).not.toBeInTheDocument();
    expect(
      within(detailCell()).getByRole('group', { name: '校验报告' }),
    ).toBeInTheDocument();
    expect(
      within(detailCell()).getByRole('button', { name: '发布（需二次认证）' }),
    ).toBeEnabled();
    expect(document.querySelector('.policy-feedback--error')).toBeNull();

    recover();
    await user.click(within(group).getByRole('button', { name: '再读一次' }));
    expect(
      await within(group).findByRole('table', { name: TABLE }),
    ).toBeInTheDocument();
    expect(bodyReads()).toEqual([
      '/api/v1/policy/versions/2',
      '/api/v1/policy/versions/2',
    ]);
  });

  it('撤销草案（没有正文）：写明发布后这类 AI 零权限，「改后」一列全是没有', async () => {
    server = policyServer({
      desired: [
        desiredBody({
          body: null,
          report: reportBody({
            retire: true,
            expansions: [
              {
                path: '(retire)',
                before: null,
                after: null,
                note: '撤销能力集：该类型回到零权限',
              },
            ],
            narrowings: [],
          }),
        }),
      ],
      versions: [versionBody(2)],
      bodies: { 2: capabilityJson() },
    });
    const group = await openDraft();
    expect(
      within(group).getByText(
        '这是一次撤销：发布后这类 AI 回到零权限，下面「现在」一列的能力都会收回。',
      ),
    ).toBeInTheDocument();
    const table = await within(group).findByRole('table', { name: TABLE });
    expect(headers(table)).toEqual(['能力', '现在生效中 v2', '改后撤销之后']);
    expect(within(table).getAllByText('去掉')).toHaveLength(3);
    expect(within(table).queryByText('新增')).not.toBeInTheDocument();
  });

  it('现在没有生效的能力集：一版正文都不读，「现在」一列全是没有，每一项都是新给的', async () => {
    server = policyServer({
      status: statusBody({ entries: [] }),
      desired: [
        desiredBody({
          body: DRAFT_CAPABILITY,
          report: reportBody({ current_version: null, narrowings: [] }),
        }),
      ],
    });
    const group = await openDraft();
    expect(
      within(group).getByText(
        '现在没有生效的能力集：发布后，下面「改后」一列的每一项都是新给的。',
      ),
    ).toBeInTheDocument();
    const table = within(group).getByRole('table', { name: TABLE });
    expect(headers(table)).toEqual([
      '能力',
      '现在没有生效版本',
      '改后这份草案',
    ]);
    expect(
      within(entry(table, '项目/Semiment')).getByText('扩大'),
    ).toBeInTheDocument();
    expect(bodyReads()).toEqual([]);
  });

  it('校验之后生效的版本又变了：写明这张表和报告照哪一版算的；基准仍是校验时那一版，列头不叫它「生效中」', async () => {
    server = policyServer({
      status: statusBody({ entries: [entryOf('capability_set', 'claude', 3)] }),
      desired: [desiredBody({ body: DRAFT_CAPABILITY })],
      versions: [versionBody(3), versionBody(2)],
      bodies: {
        2: capabilityJson(),
        3: capabilityJson((draft) => {
          draft.write.board = true;
        }),
      },
    });
    const group = await openDraft();
    expect(
      within(group).getByText(
        '生效的版本已经不是校验时的那一版（现在是 v3）：这张表和校验报告还是照 v2 算的，重新校验后再看。',
      ),
    ).toBeInTheDocument();
    const table = await within(group).findByRole('table', { name: TABLE });
    expect(headers(table)).toEqual(['能力', '现在校验时的 v2', '改后这份草案']);
    // 比的是 v2：v3 里才打开的黑板写入不在这张表的改动里。
    expect(
      within(group).getByText('1 项有改动，10 项不变'),
    ).toBeInTheDocument();
    expect(bodyReads()).toEqual(['/api/v1/policy/versions/2']);
  });

  it('校验时还没有生效版本、现在有了：同样写明，不说「现在没有生效的能力集」', async () => {
    server = policyServer({
      desired: [
        desiredBody({
          body: DRAFT_CAPABILITY,
          report: reportBody({ current_version: null, narrowings: [] }),
        }),
      ],
      versions: [versionBody(2)],
      bodies: { 2: capabilityJson() },
    });
    const group = await openDraft();
    expect(
      within(group).getByText(
        '生效的版本已经不是校验时的那一版（现在是 v2）：这张表和校验报告还是照「没有生效版本」算的，重新校验后再看。',
      ),
    ).toBeInTheDocument();
    expect(
      within(group).queryByText(/^现在没有生效的能力集/),
    ).not.toBeInTheDocument();
    expect(headers(within(group).getByRole('table', { name: TABLE }))).toEqual([
      '能力',
      '现在校验时没有生效版本',
      '改后这份草案',
    ]);
    expect(bodyReads()).toEqual([]);
  });

  it('读到的那一版不是这类 AI 的能力集：不比，说明原因，不拿别的东西凑', async () => {
    server = policyServer({
      desired: [desiredBody({ body: DRAFT_CAPABILITY })],
      versions: [versionBody(2, { subject: 'codex' })],
      bodies: { 2: capabilityJson() },
    });
    const group = await openDraft();
    expect(
      await within(group).findByText(
        '读到的 v2 不是这类 AI 的能力集，这张表列不出来；看下面的校验报告。',
      ),
    ).toBeInTheDocument();
    expect(within(group).queryByRole('table')).not.toBeInTheDocument();
  });

  it('草案正文读不出结构：说读不出、看校验报告，不猜', async () => {
    server = policyServer({
      desired: [desiredBody({ body: '不是 JSON' })],
      versions: [versionBody(2)],
      bodies: { 2: capabilityJson() },
    });
    const group = await openDraft();
    expect(
      await within(group).findByText(
        '这份草案的正文读不出结构（不是一个 JSON 对象），这张表列不出来；看下面的校验报告。',
      ),
    ).toBeInTheDocument();
    expect(within(group).queryByRole('table')).not.toBeInTheDocument();
  });

  it('还没有校验报告的草案：按现在生效的那一版比，只说新增、去掉，不标扩大、收窄', async () => {
    server = policyServer({
      desired: [
        desiredBody({
          body: DRAFT_CAPABILITY,
          status: 'pending',
          report: null,
          validated_at: null,
        }),
      ],
      versions: [versionBody(2)],
      bodies: { 2: capabilityJson() },
    });
    const group = await openDraft();
    const table = await within(group).findByRole('table', { name: TABLE });
    expect(headers(table)).toEqual(['能力', '现在生效中 v2', '改后这份草案']);
    expect(
      within(entry(table, '项目/Semiment')).getByText('新增'),
    ).toBeInTheDocument();
    expect(within(group).queryByText('扩大')).not.toBeInTheDocument();
    expect(within(group).queryByText('收窄')).not.toBeInTheDocument();
    expect(
      within(detailCell()).queryByRole('group', { name: '校验报告' }),
    ).not.toBeInTheDocument();
  });

  it('规则和 skill 的草案没有这一块：差别照旧看校验报告，也不读任何版本正文', async () => {
    server = policyServer({
      desired: [
        desiredBody({
          kind: 'rules',
          subject: 'CLAUDE.md',
          body: '# 规则',
          report: reportBody({
            kind: 'rules',
            subject: 'CLAUDE.md',
            expansions: [],
            narrowings: [],
          }),
        }),
      ],
      versions: [versionBody(2)],
    });
    window.location.hash = '#view=policy&panel=pending';
    render(<PolicyBoundary />);
    await screen.findByRole('list', { name: '待处理草案' });
    expect(
      within(detailCell()).getByRole('group', { name: '校验报告' }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('group', { name: '能力差异' }),
    ).not.toBeInTheDocument();
    expect(bodyReads()).toEqual([]);
  });
});

describe('提交：正在写的和生效中那一版的差别', () => {
  /** 打开「提交」；返回右边那一格。 */
  async function openSubmit() {
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });
    await openTab(user, '提交');
    return {
      user,
      compare: screen.getByRole('region', { name: '和生效中的差别' }),
    };
  }

  const write = (value: string) =>
    fireEvent.change(submitFields().body, { target: { value } });

  const start = (version: number) =>
    screen.getByRole('button', { name: `从生效中的 v${version} 开始改` });

  it('填了已有能力集的 AI 类型：列出差别，随输入更新；只说改了什么，不标扩大、收窄；正文没写完就说明原因', async () => {
    server = policyServer({
      versions: [versionBody(2)],
      bodies: { 2: capabilityJson() },
    });
    const { user, compare } = await openSubmit();
    expect(
      within(compare).getByText(
        '先填对象（AI 类型名）：这里会列出它和生效中那一版的差别。',
      ),
    ).toBeInTheDocument();
    expect(bodyReads()).toEqual([]);

    await user.type(submitFields().subject, 'claude');
    const table = await within(compare).findByRole('table', { name: TABLE });
    expect(headers(table)).toEqual(['能力', '现在生效中 v2', '改后正在写的']);
    expect(bodyReads()).toEqual(['/api/v1/policy/versions/2']);

    write(DRAFT_CAPABILITY);
    expect(
      await within(compare).findByText('1 项有改动，10 项不变'),
    ).toBeInTheDocument();
    expect(
      within(entry(compare, '项目/Semiment')).getByText('新增'),
    ).toBeInTheDocument();
    expect(
      within(entry(compare, '项目/旧项目')).getByText('去掉'),
    ).toBeInTheDocument();
    expect(within(compare).queryByText('扩大')).not.toBeInTheDocument();
    expect(within(compare).queryByText('收窄')).not.toBeInTheDocument();
    expect(
      within(compare).getByText(
        '这张表只列改了什么；哪些算扩大、哪些算收窄，由提交后的校验报告判定。',
      ),
    ).toBeInTheDocument();

    write('{ "ai_type": ');
    expect(
      await within(compare).findByText(
        '正文还不是一段完整的 JSON：写完整之后，这里会列出差别。',
      ),
    ).toBeInTheDocument();
    expect(within(compare).queryByRole('table')).not.toBeInTheDocument();
    write('');
    expect(
      await within(compare).findByText(
        '正文还是空的：写上 JSON 之后，这里会列出它和现在的差别。',
      ),
    ).toBeInTheDocument();
    write('[]');
    expect(
      await within(compare).findByText(
        '正文不是一个 JSON 对象（要用花括号包起来）：改好之后，这里会列出差别。',
      ),
    ).toBeInTheDocument();

    // 一直只是在写：基准只读了一次，没有发出任何写请求。
    expect(bodyReads()).toEqual(['/api/v1/policy/versions/2']);
    expect(server.posts).toEqual([]);
  });

  it('「从生效中的 v2 开始改」：生效的正文排成多行放进编辑区，差别归零；不另读一遍', async () => {
    server = policyServer({
      versions: [versionBody(2)],
      bodies: { 2: capabilityJson() },
    });
    const { user, compare } = await openSubmit();
    await user.type(submitFields().subject, 'claude');
    await within(compare).findByRole('table', { name: TABLE });

    await user.click(start(2));
    const pretty = JSON.stringify(JSON.parse(capabilityJson()), null, 2);
    await waitFor(() => expect(submitFields().body).toHaveValue(pretty));
    expect(pretty.split('\n').length).toBeGreaterThan(10);
    expect(
      await within(compare).findByText('11 项都和现在一样'),
    ).toBeInTheDocument();
    // 没有一处不同：不出表，要看再列出。
    expect(within(compare).queryByRole('table')).not.toBeInTheDocument();
    expect(
      within(compare).getByRole('button', { name: '列出不变的 11 项' }),
    ).toHaveAttribute('aria-expanded', 'false');
    expect(bodyReads()).toEqual(['/api/v1/policy/versions/2']);
    expect(server.posts).toEqual([]);
  });

  it('勾了「撤销该对象」：写明发布后零权限，「改后」一列全是没有；不能再「开始改」', async () => {
    server = policyServer({
      versions: [versionBody(2)],
      bodies: { 2: capabilityJson() },
    });
    const { user, compare } = await openSubmit();
    await user.type(submitFields().subject, 'claude');
    await within(compare).findByRole('table', { name: TABLE });
    await user.click(submitFields().retire);

    expect(
      within(compare).getByText(
        '撤销：发布后这类 AI 回到零权限，下面「现在」一列的能力都会收回。',
      ),
    ).toBeInTheDocument();
    const table = within(compare).getByRole('table', { name: TABLE });
    expect(headers(table)).toEqual(['能力', '现在生效中 v2', '改后撤销之后']);
    expect(within(table).getAllByText('去掉')).toHaveLength(3);
    expect(start(2)).toBeDisabled();
  });

  it('新的 AI 类型（没有生效的能力集）：一版正文都不读，每一项都是新给的；没有「开始改」', async () => {
    server = policyServer({
      versions: [versionBody(2)],
      bodies: { 2: capabilityJson() },
    });
    const { user, compare } = await openSubmit();
    await user.type(submitFields().subject, 'gemini');
    write(DRAFT_CAPABILITY);

    expect(
      within(compare).getByText(
        '这类 AI 现在没有生效的能力集：发布后，下面「改后」一列的每一项都是新给的。',
      ),
    ).toBeInTheDocument();
    const table = await within(compare).findByRole('table', { name: TABLE });
    expect(headers(table)).toEqual([
      '能力',
      '现在没有生效版本',
      '改后正在写的',
    ]);
    expect(
      await within(compare).findByText('9 项有改动，2 项不变'),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /^从生效中的/ }),
    ).not.toBeInTheDocument();
    expect(bodyReads()).toEqual([]);
  });

  it('生效版本的正文没读到：说明原因，提交不受影响；「再读一次」读到了就列出来', async () => {
    server = policyServer({
      versions: [versionBody(2)],
      bodies: { 2: capabilityJson() },
    });
    const recover = failBody(server, 2);
    const { user, compare } = await openSubmit();
    await user.type(submitFields().subject, 'claude');

    expect(
      await within(compare).findByText(
        '没读到生效中 v2 的正文，这里列不出差别；提交不受影响。',
      ),
    ).toBeInTheDocument();
    expect(
      within(submitFields().form).getByText('没读到 v2 的正文，再点一次重读。'),
    ).toBeInTheDocument();
    expect(submitFields().submit).toBeEnabled();
    expect(document.querySelector('.policy-feedback--error')).toBeNull();

    recover();
    await user.click(within(compare).getByRole('button', { name: '再读一次' }));
    expect(
      await within(compare).findByRole('table', { name: TABLE }),
    ).toBeInTheDocument();
  });

  it('规则和 skill：右边不列差别、不预读；有生效版本时照样能「从生效中的版本开始改」，规则正文原样放进去', async () => {
    const markdown = '# 规则\n\n- 一条';
    server = policyServer({
      status: statusBody({
        entries: [
          entryOf('capability_set', 'claude', 2),
          entryOf('rules', 'CLAUDE.md', 5),
        ],
      }),
      versions: [
        versionBody(5, { kind: 'rules', subject: 'CLAUDE.md' }),
        versionBody(2),
      ],
      bodies: { 5: markdown, 2: capabilityJson() },
    });
    const { user, compare } = await openSubmit();
    await user.selectOptions(submitFields().kind, 'rules');
    await user.type(submitFields().subject, 'CLAUDE.md');

    expect(
      within(compare).getByText(
        '这里只对能力集列出「现在／改后」。规则和 skill 的差别，提交后在校验报告里逐项列出。',
      ),
    ).toBeInTheDocument();
    expect(within(compare).queryByRole('table')).not.toBeInTheDocument();
    expect(bodyReads()).toEqual([]);

    await user.click(start(5));
    await waitFor(() => expect(submitFields().body).toHaveValue(markdown));
    expect(bodyReads()).toEqual(['/api/v1/policy/versions/5']);
    expect(server.posts).toEqual([]);
  });
});
