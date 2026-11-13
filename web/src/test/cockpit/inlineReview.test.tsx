import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createFixtureApi } from '../../../dev/fixtureApi';
import { CSRF_TOKEN } from '../../../dev/fixtures/ids';
import { createSeenTracker } from '../../cockpit/lastSeen';
import { createLaterStore } from '../../cockpit/later';
import { VERDICT_TEXT } from '../../cockpit/review';
import { installFactorRuntime } from '../../factorApi';
import { factorState } from '../../factorModel';
import { createSingleKeysPreference } from '../../keys/preference';
import { installPolicyWriter } from '../../policyApi';
import { desiredList, policyStatus } from '../../policyModel';
import type { ListedDesired } from '../../policyModel';
import { ApiError } from '../../sessionHttp';
import type { View } from '../../types';
import { validateWorkbench } from '../../validate';
import { KeysHarness } from '../keys/KeysHarness';
import { CockpitHarness } from './CockpitHarness';
import { DESIRED_URL, NOW, read, toResponse } from './cockpitFixtures';

/**
 * 驾驶舱队列里就地批准、退回策略草案（T-200）：整条路走真的——驾驶舱视图、全站那一份写动作、
 * 认证窗口、全站按键监听；后面接的是开发假数据的应答器（读走 fetch，写走受控会话写者，
 * 二次认证走受控会话的因子入口），所以发布要验证码、5 分钟里不用再输、生效内容变了会被挡下，
 * 都是应答器按真实接口的规矩算出来的，不是测试里摆的。
 */

const WAIT = { timeout: 4000 };
const STATUS_URL = '/api/v1/policy';
const ACTIVATE = '/api/v1/policy/activate';
const VALIDATE = '/api/v1/policy/validate';
const WITHDRAW = '/api/v1/policy/withdraw';
/** 假数据那份 AI 提案在队列里的标题。 */
const PROPOSAL = '能力集 claude';
/** 测试里另交的那份主人自己的草案。 */
const CODEX = '能力集 codex';
/** 测试用的一次性验证码：只在假数据应答器里有效。 */
const CODE = '246810';
const MINUTE = 60_000;

/** 能力集 codex 多读八个项目：八条扩大项，超过就地细节平时列的六条。 */
const CODEX_WIDE = JSON.stringify({
  ai_type: 'codex',
  read: {
    project_whitelist: [
      '项目/Studiolo',
      ...Array.from({ length: 8 }, (_, index) => `项目/新增${index + 1}`),
    ],
    deny_by_default: true,
  },
  write: {
    board: true,
    board_projects: ['项目/Studiolo'],
    content_patch_only: true,
    content_direct: false,
  },
  policy: { propose_desired: false, activate: false },
  hard_bound_skills: [],
});

function memory() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
    removeItem: (key: string) => {
      values.delete(key);
    },
  };
}

const failure = (error: string, status: number) =>
  new Response(JSON.stringify({ error }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });

interface WorldOptions {
  /** 另交一份主人自己的草案（能力集 codex），排在提案后面一位。 */
  second?: boolean;
  /** 二次认证入口：装上（缺省）、本入口没有验证码路径、本页没有受控会话。 */
  factor?: 'ready' | 'no-path' | 'none';
}

let uninstalls: (() => void)[] = [];

/** 一个假数据的世界：应答器、请求记录，和几个让接口出状况的开关。 */
function world(options: WorldOptions = {}) {
  let now = NOW;
  const api = createFixtureApi({ now: () => now });
  const post = (url: string, body: unknown) =>
    api.handle({
      method: 'POST',
      url,
      headers: { 'x-studiolo-request': '1', 'x-csrf-token': CSRF_TOKEN },
      body,
    });
  if (options.second) {
    // 09:13 交的：紧跟在 09:12 的提案后面，排在黑板任务前面。
    now = NOW - (11 * 60 + 18) * MINUTE;
    const submitted = post('/api/v1/policy/desired', {
      kind: 'capability_set',
      subject: 'codex',
      body: CODEX_WIDE,
    });
    if (submitted.status !== 200) throw new Error('假数据没收下第二份草案');
    now = NOW;
  }
  const state = {
    posts: [] as { path: string; body: unknown }[],
    requests: [] as string[],
    /** 读了几次服务端的因子状态（核对、认证窗口、发布成功后各一次）。 */
    factorReads: 0,
    /** 提交了几次验证码。 */
    stepUps: 0,
    /** 读「现在生效的内容」时出错。 */
    statusDown: false,
    /** 读草案列表时出错。 */
    draftsDown: false,
    /** 策略状态说本入口有没有验证码路径。 */
    l3: true,
  };
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      state.requests.push(url);
      if (url === STATUS_URL && state.statusDown)
        return failure('生效中的内容读取不可用。', 503);
      if (url === DESIRED_URL && state.draftsDown)
        return failure('策略草案读取不可用。', 503);
      const outcome = api.handle({
        method: 'GET',
        url,
        headers: { 'x-studiolo-request': '1' },
      });
      return url === STATUS_URL && !state.l3 && outcome.status === 200
        ? toResponse({
            status: 200,
            body: { ...(outcome.body as object), l3_available: false },
          })
        : toResponse(outcome);
    }),
  );
  uninstalls.push(
    installPolicyWriter(async (path, body) => {
      state.posts.push({ path, body });
      const outcome = post(path, body);
      if (outcome.status >= 400) {
        const details = outcome.body as { error?: string } | undefined;
        throw new ApiError(
          details?.error ?? '请求失败。',
          outcome.status,
          outcome.body,
        );
      }
      return outcome.body;
    }),
  );
  if (options.factor !== 'none')
    uninstalls.push(
      installFactorRuntime({
        state: async () => {
          state.factorReads += 1;
          return options.factor === 'no-path'
            ? null
            : factorState(read(api, '/api/v1/auth/state'));
        },
        submit: async (action) => {
          if (action.kind !== 'step-up') return { kind: 'stopped' };
          state.stepUps += 1;
          const outcome = post('/api/v1/auth/step-up', { code: action.code });
          return outcome.status === 200
            ? {
                kind: 'step-up',
                recent: outcome.body as {
                  verified_at_ms: number;
                  expires_at_ms: number;
                },
              }
            : {
                kind: 'failed',
                message: '验证或请求未通过。',
                status: outcome.status,
              };
        },
      }),
    );
  const drafts = () => desiredList(read(api, DESIRED_URL));
  const draft = (subject: string): ListedDesired =>
    drafts().find((entry) => entry.subject === subject)!;
  return {
    api,
    state,
    post,
    drafts,
    draft,
    workbench: validateWorkbench(read(api, '/api/workbench')),
    effective: () => policyStatus(read(api, STATUS_URL)).effective_hash,
    /** 某个地址读了几次。 */
    reads: (url: string) =>
      state.requests.filter((request) => request === url).length,
    /** 写接口按顺序发了哪几个。 */
    paths: () => state.posts.map((entry) => entry.path),
  };
}

const queue = () => screen.getByRole('region', { name: '等你处理' });
const rows = () =>
  Array.from(queue().querySelectorAll<HTMLElement>('.cockpit-queue__row'));
const titles = () =>
  rows().map(
    (row) => row.querySelector('.cockpit-queue__select')!.textContent ?? '',
  );
const select = (index: number) =>
  rows()[index].querySelector('.cockpit-queue__select') as HTMLElement;
const selectedIndex = () =>
  rows().findIndex((row) => row.hasAttribute('data-selected'));
const selectedRow = () => rows()[selectedIndex()];
const primary = () =>
  queue().querySelector<HTMLElement>('[data-queue-primary]');
const count = () =>
  queue().querySelector('.cockpit-queue__count') as HTMLElement;
const outcome = () =>
  queue().querySelector<HTMLElement>('.cockpit-queue__outcome');
const countdown = () =>
  queue().querySelector<HTMLElement>('.cockpit-queue__auth');
const approve = (label: string) =>
  within(queue()).getByRole('button', { name: `${label}，批准并发布` });
const reject = (label: string, verb: '退回' | '撤回' = '退回') =>
  within(queue()).getByRole('button', { name: `${label}，${verb}` });
const validateButton = () =>
  queue().querySelector<HTMLElement>('[data-review-validate]');
const preview = (title: string) =>
  screen.findByRole('group', { name: `执行预览 ${title}` }, WAIT);
const noPreview = () => screen.queryByRole('group', { name: /^执行预览/ });
const press = (key: string) =>
  fireEvent.keyDown(document.activeElement ?? document.body, { key });
/** 巨号给出的总数。 */
const total = () => Number(count().textContent);
const settled = (expected: number) =>
  waitFor(() => expect(count().textContent).toBe(String(expected)), WAIT);

/** 打开驾驶舱，等三路都读完；返回这个世界、用户，和开始时的总数。 */
async function open(options: WorldOptions = {}) {
  const w = world(options);
  const onGo = vi.fn<(view: View) => void>();
  render(
    <KeysHarness preference={createSingleKeysPreference(memory())} onGo={onGo}>
      <CockpitHarness
        workbench={w.workbench}
        refreshing={false}
        diagnostics={0}
        seen={createSeenTracker({ storage: null, now: Date.now })}
        later={createLaterStore({ storage: memory(), now: () => Date.now() })}
        onOpen={vi.fn()}
        onShowDiagnostics={vi.fn()}
      />
    </KeysHarness>,
  );
  await waitFor(() => expect(count().textContent).toMatch(/^\d+$/), WAIT);
  return { w, user: userEvent.setup(), onGo, initial: total() };
}

type Opened = Awaited<ReturnType<typeof open>>;

/** 在认证窗口里输入验证码并提交。 */
async function authenticate(user: Opened['user']) {
  const dialog = await screen.findByRole(
    'dialog',
    { name: '二次认证后继续' },
    WAIT,
  );
  // 窗口先核对二次认证可不可用，核对完才有输入框。
  await user.type(
    await within(dialog).findByLabelText('验证码', {}, WAIT),
    CODE,
  );
  await user.click(
    within(dialog).getByRole('button', { name: '完成认证并继续' }),
  );
  return dialog;
}

/** 等队列标题下面出现结果那一句。 */
async function result() {
  await waitFor(() => expect(outcome()).not.toBeNull(), WAIT);
  return outcome()!;
}

beforeEach(() => {
  // 假数据的「此刻」是 2026-09-22 20:31：本机时钟拨到同一刻。
  vi.spyOn(Date, 'now').mockReturnValue(NOW);
});

afterEach(() => {
  for (const uninstall of uninstalls) uninstall();
  uninstalls = [];
});

describe('就地审批的两个按钮', () => {
  it('选中的是策略草案才有「批准并发布」「退回」；黑板任务、工作台待办的行没有', async () => {
    const { w, user } = await open();
    expect(titles()[0]).toBe(PROPOSAL);
    expect(approve(PROPOSAL)).toBeEnabled();
    expect(approve(PROPOSAL)).toHaveTextContent('批准并发布');
    expect(reject(PROPOSAL)).toHaveTextContent('退回');
    // 两个都不是实心按钮：全页的实心主按钮还是只有「去处理」那一个。
    expect(document.querySelectorAll('.ui-button--primary')).toHaveLength(1);
    // 没按之前：不核对、不出预览、不写。
    expect(noPreview()).toBeNull();
    expect(w.reads(STATUS_URL)).toBe(0);
    expect(w.state.factorReads).toBe(0);
    expect(w.state.posts).toEqual([]);

    await user.click(select(1));
    expect(selectedRow()).toHaveAttribute('data-source', 'board');
    expect(queue().querySelector('[data-queue-approve]')).toBeNull();
    expect(queue().querySelector('[data-queue-reject]')).toBeNull();

    await user.click(select(rows().length - 1));
    expect(selectedRow()).toHaveAttribute('data-source', 'inbox');
    expect(queue().querySelector('[data-queue-approve]')).toBeNull();
    expect(queue().querySelector('[data-queue-reject]')).toBeNull();
  });

  it('自己交的草案上，退回叫「撤回」', async () => {
    await open({ second: true });
    expect(titles().slice(0, 2)).toEqual([PROPOSAL, CODEX]);
    fireEvent.click(select(1));
    expect(reject(CODEX, '撤回')).toHaveTextContent('撤回');
    expect(approve(CODEX)).toBeEnabled();
  });
});

describe('批准并发布', () => {
  it('先核对两样、展开执行预览，确认才发请求；被要求二次认证就地输验证码，原样重试同一个请求', async () => {
    const { w, user, initial } = await open();
    const proposal = w.draft('claude');
    const base = proposal.report!.base_effective_hash;
    expect(base).toBe(w.effective());
    const listReads = w.reads(DESIRED_URL);

    await user.click(approve(PROPOSAL));
    const box = await preview(`发布 ${PROPOSAL}`);
    // 核对：现在生效的内容、二次认证状态各读一次；还没有任何写入。
    expect(w.reads(STATUS_URL)).toBe(1);
    expect(w.state.factorReads).toBe(1);
    expect(w.state.posts).toEqual([]);
    expect(box).toHaveClass('policy-preview');
    expect(box).toHaveFocus();
    expect(box).toHaveTextContent(proposal.id);
    expect(box).toHaveTextContent(`生效中摘要 ${base.slice(0, 12)}`);
    expect(box).toHaveTextContent('3 项，上面已逐条列出');
    expect(box).toHaveTextContent('不可撤回');
    // 眼下没有近期认证：写的是缺省那一句。
    expect(box).toHaveTextContent(
      '需要二次认证（一次性验证码）：服务端要求时会弹出窗口请你输入，通过后才会发布。',
    );
    expect(approve(PROPOSAL)).toHaveAttribute('aria-expanded', 'true');
    expect(approve(PROPOSAL)).toHaveAttribute('aria-controls', box.id);

    await user.click(within(box).getByRole('button', { name: '确认发布' }));
    const dialog = await screen.findByRole(
      'dialog',
      { name: '二次认证后继续' },
      WAIT,
    );
    // 请求带的是校验报告的基准摘要：人看到的扩大项是照哪一份算的，就按哪一份提交。
    const request = {
      path: ACTIVATE,
      body: { desired_id: proposal.id, expected_effective_hash: base },
    };
    expect(w.state.posts).toEqual([request]);
    // 被挡下：什么都没发生。
    expect(total()).toBe(initial);
    expect(titles()[0]).toBe(PROPOSAL);
    expect(noPreview()).toBeNull();
    expect(dialog).toHaveTextContent(`本次动作：发布 ${PROPOSAL}`);
    expect(w.reads(DESIRED_URL)).toBe(listReads);
    // 认证窗口自己读了一次因子状态；驾驶舱这边在窗口开着时不读。
    await waitFor(() => expect(w.state.factorReads).toBe(2), WAIT);

    await authenticate(user);
    const line = await result();
    expect(line).toHaveAttribute('role', 'status');
    expect(line.textContent).toMatch(
      /^能力集 claude：已发布为 v\d+，生效中摘要 [0-9a-f]{12}。$/,
    );
    // 认证后重试的和第一次逐字节相同。
    expect(w.state.posts).toEqual([request, request]);
    expect(w.state.stepUps).toBe(1);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(w.effective()).not.toBe(base);
    expect(w.draft('claude').status).toBe('activated');

    // 那一行不在队列里了：总数少一，草案列表只重读了一次。
    await settled(initial - 1);
    expect(titles()).not.toContain(PROPOSAL);
    expect(w.reads(DESIRED_URL)).toBe(listReads + 1);
    // 改选顶上来的那一行，焦点交给它的主按钮，接着审下一件。
    expect(selectedIndex()).toBe(0);
    await waitFor(() => expect(primary()).toHaveFocus(), WAIT);
    // 发布成功后读了一次近期认证（窗口已经关了）；队列里没有策略草案了，倒计时不写。
    await waitFor(() => expect(w.state.factorReads).toBe(3), WAIT);
    expect(countdown()).toBeNull();
  });

  it('一次验证码 5 分钟里连着批：第二份先重新校验，再批就不用再输', async () => {
    const { w, user, initial } = await open({ second: true });
    expect(titles().slice(0, 2)).toEqual([PROPOSAL, CODEX]);
    const codex = w.draft('codex');

    await user.click(approve(PROPOSAL));
    await user.click(
      within(await preview(`发布 ${PROPOSAL}`)).getByRole('button', {
        name: '确认发布',
      }),
    );
    await authenticate(user);
    await result();
    await settled(initial - 1);
    const moved = w.effective();

    // 顶上来的就是第二份草案：选中、焦点在它的主按钮上，标题下面写着认证还剩多久。
    expect(titles()[0]).toBe(CODEX);
    expect(selectedIndex()).toBe(0);
    await waitFor(() => expect(primary()).toHaveFocus(), WAIT);
    await waitFor(() => expect(countdown()).not.toBeNull(), WAIT);
    expect(countdown()!.textContent).toMatch(
      /^二次认证还有效 (5 分|4 分 5\d 秒)：这段时间里再批准，一般不用重新输入验证码（以服务端为准）。$/,
    );
    expect(countdown()).not.toHaveAttribute('role');

    // 它的校验报告是照发布之前的内容算的：不让批，焦点交给「重新校验」。
    expect(codex.report!.base_effective_hash).not.toBe(moved);
    await user.click(approve(CODEX));
    expect(
      await within(selectedRow()).findByText(VERDICT_TEXT.stale, {}, WAIT),
    ).toHaveAttribute('role', 'status');
    expect(noPreview()).toBeNull();
    expect(validateButton()).toHaveTextContent('重新校验');
    expect(validateButton()).toHaveFocus();
    expect(w.paths()).toEqual([ACTIVATE, ACTIVATE]);

    await user.click(validateButton()!);
    expect(
      await within(selectedRow()).findByText(
        `已重新校验 ${CODEX}（草案 ${codex.id.slice(0, 12)}）：已校验，可发布。`,
        {},
        WAIT,
      ),
    ).toHaveAttribute('role', 'status');
    expect(w.state.posts.at(-1)).toEqual({
      path: VALIDATE,
      body: { desired_id: codex.id },
    });
    // 校验完：那一句「不能直接批准」和「重新校验」都撤掉，焦点交给「批准并发布」。
    expect(within(queue()).queryByText(VERDICT_TEXT.stale)).toBeNull();
    expect(validateButton()).toBeNull();
    await waitFor(() => expect(approve(CODEX)).toHaveFocus(), WAIT);
    expect(w.draft('codex').report!.base_effective_hash).toBe(moved);

    // 再批：预览照实说认证还有效；确认后直接发布，不弹认证窗口。
    await user.click(approve(CODEX));
    const box = await preview(`发布 ${CODEX}`);
    expect(box).toHaveTextContent(
      /二次认证还有效约 (5 分|4 分 \d+ 秒)（以服务端为准）：确认后直接发布/,
    );
    expect(box).toHaveTextContent('8 项，上面已逐条列出');
    await user.click(within(box).getByRole('button', { name: '确认发布' }));
    await waitFor(
      () =>
        expect(outcome()?.textContent).toMatch(
          /^能力集 codex：已发布为 v\d+，生效中摘要 [0-9a-f]{12}。$/,
        ),
      WAIT,
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(w.state.stepUps).toBe(1);
    expect(w.state.posts.at(-1)).toEqual({
      path: ACTIVATE,
      body: { desired_id: codex.id, expected_effective_hash: moved },
    });
    expect(w.paths()).toEqual([ACTIVATE, ACTIVATE, VALIDATE, ACTIVATE]);
    await settled(initial - 2);
    // 队列里没有策略草案了：倒计时撤掉。
    expect(countdown()).toBeNull();
  });

  it('预览开着时，就地细节全部列出，不截到六条；收起又回到六条', async () => {
    const { user } = await open({ second: true });
    await user.click(select(1));
    const detail = () =>
      selectedRow().querySelector<HTMLElement>('.cockpit-detail')!;
    const expansions = () =>
      within(
        within(detail())
          .getByRole('heading', { level: 3, name: /^扩大/ })
          .closest('section')!,
      ).getAllByRole('listitem');
    expect(expansions()).toHaveLength(6);
    expect(detail()).toHaveTextContent('还有 2 条没列出来。');

    await user.click(approve(CODEX));
    const box = await preview(`发布 ${CODEX}`);
    expect(expansions()).toHaveLength(8);
    expect(detail()).not.toHaveTextContent('没列出来');

    await user.click(within(box).getByRole('button', { name: '取消' }));
    expect(noPreview()).toBeNull();
    expect(expansions()).toHaveLength(6);
    // 取消之后焦点回到打开它的按钮。
    expect(approve(CODEX)).toHaveFocus();
  });

  it('确认之前生效内容被别处改了：服务端挡下，照实报错，什么都没发布', async () => {
    const { w, user, initial } = await open({ second: true });
    const proposal = w.draft('claude');
    await user.click(approve(PROPOSAL));
    const box = await preview(`发布 ${PROPOSAL}`);

    // 预览开着的这一会儿，别处发布了另一份草案（应答器里直接做：先认证，再发布 codex 那份）。
    expect(w.post('/api/v1/auth/step-up', { code: CODE }).status).toBe(200);
    expect(
      w.post(ACTIVATE, {
        desired_id: w.draft('codex').id,
        expected_effective_hash: w.effective(),
      }).status,
    ).toBe(200);

    await user.click(within(box).getByRole('button', { name: '确认发布' }));
    const alert = await within(queue()).findByRole('alert', {}, WAIT);
    expect(alert).toHaveTextContent(
      'effective 已变化：请刷新并重新校验后再发布。',
    );
    expect(w.state.posts).toEqual([
      {
        path: ACTIVATE,
        body: {
          desired_id: proposal.id,
          expected_effective_hash: proposal.report!.base_effective_hash,
        },
      },
    ]);
    expect(w.draft('claude').status).toBe('pending');
    // 重读之后别处发布掉的那一份不在了；这一份还在、还选中，报错就写在它自己那一行里。
    await settled(initial - 1);
    expect(titles()[0]).toBe(PROPOSAL);
    expect(selectedIndex()).toBe(0);
    expect(within(selectedRow()).getByRole('alert')).toHaveTextContent(
      'effective 已变化',
    );
    expect(outcome()).toBeNull();

    // 再按批准：报告旧了，先重新校验。
    await user.click(approve(PROPOSAL));
    expect(
      await within(selectedRow()).findByText(VERDICT_TEXT.stale, {}, WAIT),
    ).toBeInTheDocument();
    expect(noPreview()).toBeNull();
  });

  it('被要求认证时取消：不发布、不重试，提示照实改成「已取消」', async () => {
    const { w, user, initial } = await open();
    await user.click(approve(PROPOSAL));
    await user.click(
      within(await preview(`发布 ${PROPOSAL}`)).getByRole('button', {
        name: '确认发布',
      }),
    );
    const dialog = await screen.findByRole(
      'dialog',
      { name: '二次认证后继续' },
      WAIT,
    );
    await user.click(
      await within(dialog).findByRole('button', { name: '取消，不发布' }, WAIT),
    );
    await waitFor(
      () => expect(screen.queryByRole('dialog')).not.toBeInTheDocument(),
      WAIT,
    );
    expect(
      within(selectedRow()).getByText(
        `已取消二次认证：发布 ${PROPOSAL} 没有执行。`,
      ),
    ).toHaveAttribute('role', 'status');
    expect(w.paths()).toEqual([ACTIVATE]);
    expect(w.state.stepUps).toBe(0);
    expect(total()).toBe(initial);
    expect(titles()[0]).toBe(PROPOSAL);
    expect(selectedIndex()).toBe(0);
    expect(outcome()).toBeNull();
    expect(w.draft('claude').status).toBe('pending');
  });
});

describe('批准之前的核对没过：不出预览，不发请求', () => {
  it('本入口没有验证码路径（因子状态这么说）：写明这里只能查看和退回', async () => {
    const { w, user } = await open({ factor: 'no-path' });
    await user.click(approve(PROPOSAL));
    expect(
      await within(selectedRow()).findByText(VERDICT_TEXT['no-path'], {}, WAIT),
    ).toHaveAttribute('role', 'status');
    expect(noPreview()).toBeNull();
    expect(w.state.posts).toEqual([]);
    // 退回照样能用。
    expect(reject(PROPOSAL)).toBeEnabled();
    expect(validateButton()).toBeNull();
  });

  it('本入口没有验证码路径（策略状态这么说）：同一句', async () => {
    const { w, user } = await open();
    w.state.l3 = false;
    await user.click(approve(PROPOSAL));
    expect(
      await within(selectedRow()).findByText(VERDICT_TEXT['no-path'], {}, WAIT),
    ).toBeInTheDocument();
    expect(noPreview()).toBeNull();
    expect(w.state.posts).toEqual([]);
  });

  it('本页没有受控会话：请人重新登录', async () => {
    const { w, user } = await open({ factor: 'none' });
    await user.click(approve(PROPOSAL));
    expect(
      await within(selectedRow()).findByText(
        VERDICT_TEXT['no-session'],
        {},
        WAIT,
      ),
    ).toBeInTheDocument();
    expect(noPreview()).toBeNull();
    expect(w.state.posts).toEqual([]);
  });

  it('没读到现在生效的内容：照实说没能核对；再按一次重试', async () => {
    const { w, user } = await open();
    w.state.statusDown = true;
    await user.click(approve(PROPOSAL));
    const alert = await within(selectedRow()).findByRole('alert', {}, WAIT);
    expect(alert).toHaveTextContent(
      '没能核对，预览没有展开：生效中的内容读取不可用。 再按一次「批准并发布」重试。',
    );
    expect(noPreview()).toBeNull();
    expect(w.state.posts).toEqual([]);
    // 第一样没读成就停下，不接着读第二样。
    expect(w.state.factorReads).toBe(0);

    w.state.statusDown = false;
    await user.click(approve(PROPOSAL));
    await preview(`发布 ${PROPOSAL}`);
    expect(within(queue()).queryByRole('alert')).toBeNull();
    expect(w.state.posts).toEqual([]);
  });
});

describe('退回', () => {
  it('先展开确认框，确认才撤回；结果写在队列标题下面，改选顶上来的那一行', async () => {
    const { w, user, initial } = await open({ second: true });
    const proposal = w.draft('claude');
    await user.click(reject(PROPOSAL));
    const box = await preview(`退回 ${PROPOSAL}`);
    expect(box).toHaveFocus();
    expect(box).toHaveTextContent(proposal.id);
    expect(box).toHaveTextContent('AI claude');
    expect(box).toHaveTextContent('不需要一次性验证码');
    expect(reject(PROPOSAL)).toHaveAttribute('aria-expanded', 'true');
    expect(reject(PROPOSAL)).toHaveAttribute('aria-controls', box.id);
    expect(w.state.posts).toEqual([]);

    await user.click(within(box).getByRole('button', { name: '确认退回' }));
    const line = await result();
    expect(line).toHaveAttribute('role', 'status');
    expect(line).toHaveTextContent(
      `已退回 ${PROPOSAL}（草案 ${proposal.id.slice(0, 12)}）。`,
    );
    expect(w.state.posts).toEqual([
      { path: WITHDRAW, body: { desired_id: proposal.id } },
    ]);
    expect(w.draft('claude').status).toBe('withdrawn');
    await settled(initial - 1);
    expect(titles()[0]).toBe(CODEX);
    expect(selectedIndex()).toBe(0);
    await waitFor(() => expect(primary()).toHaveFocus(), WAIT);
    // 退回不要验证码：没有认证窗口，也不读因子状态；生效中的内容不变。
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(w.state.factorReads).toBe(0);
    expect(w.reads(STATUS_URL)).toBe(0);
    expect(countdown()).toBeNull();
  });

  it('做完重读没读成：不留着写之前的列表，照实显示读取失败，可重试', async () => {
    const { w, user, initial } = await open();
    await user.click(reject(PROPOSAL));
    w.state.draftsDown = true;
    await user.click(
      within(await preview(`退回 ${PROPOSAL}`)).getByRole('button', {
        name: '确认退回',
      }),
    );
    expect(
      await within(queue()).findByText(/^策略草案读取失败/, {}, WAIT),
    ).toHaveTextContent('策略草案读取不可用。');
    expect(count()).toHaveTextContent('—');
    expect(titles()).not.toContain(PROPOSAL);
    expect(w.paths()).toEqual([WITHDRAW]);

    w.state.draftsDown = false;
    await user.click(screen.getByRole('button', { name: '重试策略草案' }));
    await settled(initial - 1);
    expect(titles()).not.toContain(PROPOSAL);
  });
});

describe('键盘：A 批准、R 退回只打开确认那一步', () => {
  it('A：核对之后展开发布预览，焦点进预览框；再按一次收起，什么都没发', async () => {
    const { w } = await open();
    press('a');
    const box = await preview(`发布 ${PROPOSAL}`);
    expect(box).toHaveFocus();
    expect(w.state.posts).toEqual([]);
    press('a');
    expect(noPreview()).toBeNull();
    // 收起之后焦点在「批准并发布」上：和用鼠标点它两下一样。
    expect(approve(PROPOSAL)).toHaveFocus();
    expect(w.state.posts).toEqual([]);
  });

  it('R：展开退回的确认框；再按一次收起，什么都没发', async () => {
    const { w } = await open();
    press('r');
    const box = await preview(`退回 ${PROPOSAL}`);
    expect(box).toHaveFocus();
    expect(
      within(box).getByRole('button', { name: '确认退回' }),
    ).toBeInTheDocument();
    press('r');
    expect(noPreview()).toBeNull();
    expect(reject(PROPOSAL)).toHaveFocus();
    expect(w.state.posts).toEqual([]);
  });

  it('按键之前焦点在队列外面：A 把焦点带到「批准并发布」上，做完照样交给顶上来的那一行', async () => {
    const { user, initial } = await open();
    // 刚点过别处的按钮（比如顶栏、右边那一格），焦点留在那里。
    const elsewhere = screen.getByRole('button', { name: '去协作页看全部' });
    elsewhere.focus();
    press('a');
    const box = await preview(`发布 ${PROPOSAL}`);
    expect(box).toHaveFocus();
    // 取消：焦点回到「批准并发布」——键是替人点这个按钮，和用鼠标点它再取消一样，不回到队列外面。
    await user.click(within(box).getByRole('button', { name: '取消' }));
    expect(noPreview()).toBeNull();
    expect(approve(PROPOSAL)).toHaveFocus();

    elsewhere.focus();
    press('a');
    await user.click(
      within(await preview(`发布 ${PROPOSAL}`)).getByRole('button', {
        name: '确认发布',
      }),
    );
    await authenticate(user);
    await result();
    await settled(initial - 1);
    // 那一行不见了：焦点交给顶上来的那一行，不回到队列外面那个按钮。
    await waitFor(() => expect(primary()).toHaveFocus(), WAIT);
    expect(elsewhere).not.toHaveFocus();
  });

  it('按键之前焦点在队列外面：R 同样把焦点带到「退回」上', async () => {
    const { user } = await open();
    const elsewhere = screen.getByRole('button', { name: '去协作页看全部' });
    elsewhere.focus();
    press('r');
    const box = await preview(`退回 ${PROPOSAL}`);
    await user.click(within(box).getByRole('button', { name: '取消' }));
    expect(reject(PROPOSAL)).toHaveFocus();
  });

  it('A 和 R 互相换：同一时刻只有一个确认框', async () => {
    await open();
    press('r');
    await preview(`退回 ${PROPOSAL}`);
    press('a');
    await preview(`发布 ${PROPOSAL}`);
    expect(screen.getAllByRole('group', { name: /^执行预览/ })).toHaveLength(1);
    press('r');
    await preview(`退回 ${PROPOSAL}`);
    expect(screen.getAllByRole('group', { name: /^执行预览/ })).toHaveLength(1);
  });

  it('预览开着时 Enter 不算「去处理」；收起之后照旧', async () => {
    const { w } = await open();
    press('a');
    const box = await preview(`发布 ${PROPOSAL}`);
    expect(box).toHaveFocus();
    press('Enter');
    expect(window.location.hash).toBe('');
    expect(noPreview()).not.toBeNull();
    expect(w.state.posts).toEqual([]);

    press('a');
    expect(noPreview()).toBeNull();
    // 收起之后焦点在「批准并发布」上，Enter 是那个按钮自己的点击，不归队列；
    // 焦点不落在任何东西上时，Enter 照旧是「去处理」。
    expect(approve(PROPOSAL)).toHaveFocus();
    approve(PROPOSAL).blur();
    press('Enter');
    expect(window.location.hash).toBe(
      `#view=policy&panel=pending&item=${w.draft('claude').id}`,
    );
  });

  it('换了一行、放到稍后：预览收起——它只描述眼前这一份', async () => {
    const { w } = await open();
    press('r');
    await preview(`退回 ${PROPOSAL}`);
    press('j');
    expect(noPreview()).toBeNull();
    expect(selectedIndex()).toBe(1);
    press('k');
    expect(selectedIndex()).toBe(0);
    expect(noPreview()).toBeNull();

    press('a');
    await preview(`发布 ${PROPOSAL}`);
    press('h');
    expect(noPreview()).toBeNull();
    expect(
      within(queue()).getByRole('list', { name: '稍后再看' }),
    ).toHaveTextContent(PROPOSAL);
    expect(w.state.posts).toEqual([]);
  });

  it('选中的不是策略草案：A、R 没反应，原样放行', async () => {
    const { w } = await open();
    press('j');
    expect(selectedRow()).toHaveAttribute('data-source', 'board');
    press('a');
    press('r');
    expect(noPreview()).toBeNull();
    expect(w.reads(STATUS_URL)).toBe(0);
    expect(w.state.factorReads).toBe(0);
    expect(w.state.posts).toEqual([]);
  });

  it('「G R」是去规则页，不落到退回上', async () => {
    const { onGo } = await open();
    press('g');
    press('r');
    expect(onGo).toHaveBeenCalledWith('rules');
    expect(noPreview()).toBeNull();
  });

  it('键位提示：选中策略草案时多写 A 批准、R 退回；选中别的行不写', async () => {
    await open();
    const hints = () =>
      within(within(queue()).getByRole('list', { name: '这一格的键盘快捷键' }))
        .getAllByRole('listitem')
        .map((item) => item.textContent);
    expect(hints()).toEqual([
      'JK上下移动',
      '↵去处理',
      'H稍后',
      'A批准',
      'R退回',
      'G跳到别的视图',
    ]);
    press('j');
    expect(hints()).toEqual([
      'JK上下移动',
      '↵去处理',
      'H稍后',
      'G跳到别的视图',
    ]);
  });
});
