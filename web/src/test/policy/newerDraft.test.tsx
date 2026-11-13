import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';
import { PolicyBoundary } from '../../PolicyBoundary';
import { COMMAND_TARGETS } from '../../commands/focusTarget';
import { ApiError } from '../../sessionHttp';
import {
  HASH,
  desiredBody,
  policyServer,
  type PolicyServer,
  type PolicyServerOptions,
} from './policyFixtures';

/**
 * 同一对象有几份未发布草案（T-181／BUG-031）：队列写出草案号并标「最新」「较早」。同一提交者还有更新的，
 * 较早那份标「较早，不能发布」、发布按钮不可点；别的提交者更新的不挡（用户 2026-10-16 定），执行预览
 * 写明发的是哪一份、会一并取代哪几份、哪几份照旧待处理。判定只照接口给的 newer_open、blocked_by，
 * 选中的记录每次从新列表取。
 */

const OLDEST = '01JZ00000999XKF8AD4CM26CRT';
const OLDER = '01JZ00000AAAXKF8AD4CM26CRT';
const NEWEST = '01JZ00000BBBXKF8AD4CM26CRT';
const MOVED = 'c'.repeat(64);

let server: PolicyServer | undefined;
afterEach(() => {
  server?.uninstall();
  server = undefined;
});

function queueCell() {
  const element = document.querySelector<HTMLElement>(
    `[data-command-target="${COMMAND_TARGETS.policyQueue}"]`,
  );
  if (!element) throw new Error('缺少待处理草案格');
  return element;
}

function queueItems() {
  return within(screen.getByRole('list', { name: '待处理草案' })).getAllByRole(
    'listitem',
  );
}

function picks() {
  return within(screen.getByRole('list', { name: '待处理草案' })).getAllByRole(
    'button',
    { name: '能力集 claude' },
  );
}

function publishButton() {
  return screen.getByRole('button', { name: '发布（需二次认证）' });
}

function preview() {
  return screen.getByRole('group', { name: /^执行预览/ });
}

/** 发布照发：回一份新版本（认证已在别处验过，这里只看界面怎么说）。 */
function publishes(path: string, body: unknown) {
  if (path === '/api/v1/policy/activate')
    return {
      activation: {
        version: 3,
        kind: 'capability_set',
        subject: 'claude',
        retired: false,
        body_sha256: HASH,
        effective_hash: MOVED,
        previous_effective_hash: HASH,
        expansions: [],
        desired_id: (body as { desired_id: string }).desired_id,
        rollback_of: null,
        cleared_type_revocation: false,
        activated_by: 'owner',
      },
      audit_confirmed: true,
    };
  throw new ApiError('未知', 404);
}

async function openPanel(options: PolicyServerOptions) {
  server = policyServer(options);
  const user = userEvent.setup();
  render(<PolicyBoundary />);
  await screen.findByRole('table', { name: '生效中的各项' });
  return { user, server };
}

/** 与生产同形：同一提交者两轮同名草案都待处理，较早那份被最新那份挡住。 */
const twoRounds: PolicyServerOptions = {
  desired: [desiredBody({ id: NEWEST }), desiredBody({ id: OLDER })],
  newer: { [OLDER]: [NEWEST] },
  blocked: { [OLDER]: [NEWEST] },
};

/** 主人的草案之后 AI 又交了一份：先后照旧，但不同提交者互不阻挡，服务端不挡较早那份。 */
const agentAfterOwner: PolicyServerOptions = {
  desired: [
    desiredBody({
      id: NEWEST,
      submitter_kind: 'agent',
      agent_type: 'claude',
      agent_instance: '01M50KWFNX6A1QBV8X1G3KKTH2',
      status: 'pending',
    }),
    desiredBody({ id: OLDER }),
  ],
  newer: { [OLDER]: [NEWEST] },
};

describe('同一对象几份未发布草案（T-181）', () => {
  it('队列给每份写出草案号前 12 位并标「最新」「较早」；同名按钮靠描述区分', async () => {
    await openPanel(twoRounds);
    const items = queueItems();
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent(NEWEST.slice(0, 12));
    expect(items[0]).toHaveTextContent('最新');
    expect(items[1]).toHaveTextContent(OLDER.slice(0, 12));
    expect(items[1]).toHaveTextContent('较早，不能发布');
    const [newest, older] = picks();
    expect(newest).toHaveAccessibleDescription(/最新/);
    expect(older).toHaveAccessibleDescription(/较早，不能发布/);
  });

  it('同一提交者还有更新的：选中较早那份，说明挡住它的是哪份，发布按钮不可点，不出执行预览', async () => {
    const { user, server } = await openPanel(twoRounds);
    await user.click(picks()[1]);
    expect(
      screen.getByText(
        `同一提交者还有更新的未发布草案（${NEWEST.slice(0, 12)}）：只能发布其中最新的那一份；要发这份，先在「策略」页的待处理草案里撤回更新的。`,
      ),
    ).toBeInTheDocument();
    expect(publishButton()).toBeDisabled();
    expect(
      screen.queryByRole('group', { name: /^执行预览/ }),
    ).not.toBeInTheDocument();
    expect(server.posts).toEqual([]);
  });

  it('选中最新那份：先说会取代较早那份；执行预览写出完整草案号与「同时取代」；成功提示补一句', async () => {
    const { user, server } = await openPanel({
      ...twoRounds,
      write: publishes,
    });
    await user.click(picks()[0]);
    expect(
      screen.getByText(
        '同一对象另有 1 份较早的未发布草案：发布这一份会把它们标为已取代。',
      ),
    ).toBeInTheDocument();
    await user.click(publishButton());
    expect(preview()).toHaveTextContent(NEWEST);
    expect(preview()).toHaveTextContent(
      `同一对象比它早的 1 份未发布草案（${OLDER.slice(0, 12)}）：发布后标为已取代，不能再发布`,
    );
    expect(preview()).not.toHaveTextContent('仍待处理');
    await user.click(
      within(preview()).getByRole('button', { name: '确认发布' }),
    );
    await waitFor(() =>
      expect(server.posts).toEqual([
        {
          path: '/api/v1/policy/activate',
          body: { desired_id: NEWEST, expected_effective_hash: HASH },
        },
      ]),
    );
    expect(
      await within(queueCell()).findByText(
        /已发布为 v3.*同一对象比它早的未发布草案已一并标为已取代/,
      ),
    ).toHaveAttribute('role', 'status');
  });

  it('更新的那份是别的提交者交的：较早那份只标「较早」、照样能发；预览红字写明它照旧待处理，成功提示也说', async () => {
    const { user, server } = await openPanel({
      ...agentAfterOwner,
      write: publishes,
    });
    const items = queueItems();
    expect(items[0]).toHaveTextContent('最新');
    expect(items[1]).toHaveTextContent('较早');
    expect(items[1]).not.toHaveTextContent('不能发布');
    await user.click(picks()[1]);
    expect(
      screen.getByText(
        '同一对象还有 1 份更新的未发布草案，是别的提交者交的：发布这一份不会取代它们，它们照旧待处理。',
      ),
    ).toBeInTheDocument();
    expect(publishButton()).toBeEnabled();
    await user.click(publishButton());
    expect(preview()).toHaveTextContent(OLDER);
    expect(preview()).toHaveTextContent('无：同一对象没有比它早的未发布草案');
    const remaining = within(preview()).getByText(
      `同一对象还有 1 份比它更新的未发布草案，是别的提交者交的（${NEWEST.slice(0, 12)}，AI claude）：这次发的是较早的这一份，它们不会被取代，发布后照旧等你处理`,
    );
    expect(remaining).toHaveClass('is-signal');
    // 确认按钮也点明发的是较早的这一份。
    expect(
      within(preview()).queryByRole('button', { name: '确认发布' }),
    ).not.toBeInTheDocument();
    await user.click(
      within(preview()).getByRole('button', {
        name: '确认发布这份较早的草案',
      }),
    );
    await waitFor(() =>
      expect(server.posts).toEqual([
        {
          path: '/api/v1/policy/activate',
          body: { desired_id: OLDER, expected_effective_hash: HASH },
        },
      ]),
    );
    expect(
      await within(queueCell()).findByText(
        /已发布为 v3.*同一对象更新的 1 份未发布草案照旧待处理/,
      ),
    ).toHaveAttribute('role', 'status');
  });

  it('夹在同一类 AI 两份提案中间的主人草案：最早那份被挡；主人那份能发，预览与成功提示同时写取代和仍待处理', async () => {
    const proposal = (id: string) =>
      desiredBody({
        id,
        submitter_kind: 'agent',
        agent_type: 'claude',
        agent_instance: '01M50KWFNX6A1QBV8X1G3KKTH2',
        status: 'pending',
      });
    const { user, server } = await openPanel({
      desired: [proposal(NEWEST), desiredBody({ id: OLDER }), proposal(OLDEST)],
      newer: { [OLDER]: [NEWEST], [OLDEST]: [OLDER, NEWEST] },
      blocked: { [OLDEST]: [NEWEST] },
      write: publishes,
    });
    const items = queueItems();
    expect(items[0]).toHaveTextContent('最新');
    expect(items[1]).toHaveTextContent('较早');
    expect(items[1]).not.toHaveTextContent('不能发布');
    expect(items[2]).toHaveTextContent('较早，不能发布');
    await user.click(picks()[1]);
    expect(
      screen.getByText(
        '同一对象另有 1 份较早的未发布草案：发布这一份会把它们标为已取代。同一对象还有 1 份更新的未发布草案，是别的提交者交的：发布这一份不会取代它们，它们照旧待处理。',
      ),
    ).toBeInTheDocument();
    await user.click(publishButton());
    expect(preview()).toHaveTextContent(
      `同一对象比它早的 1 份未发布草案（${OLDEST.slice(0, 12)}）：发布后标为已取代，不能再发布`,
    );
    expect(preview()).toHaveTextContent(
      `同一对象还有 1 份比它更新的未发布草案，是别的提交者交的（${NEWEST.slice(0, 12)}，AI claude）`,
    );
    await user.click(
      within(preview()).getByRole('button', {
        name: '确认发布这份较早的草案',
      }),
    );
    await waitFor(() =>
      expect(server.posts).toEqual([
        {
          path: '/api/v1/policy/activate',
          body: { desired_id: OLDER, expected_effective_hash: HASH },
        },
      ]),
    );
    expect(
      await within(queueCell()).findByText(
        /已发布为 v3.*；同一对象比它早的未发布草案已一并标为已取代；同一对象更新的 1 份未发布草案照旧待处理/,
      ),
    ).toHaveAttribute('role', 'status');
  });

  it('只有一份草案时不标先后，预览写「同时取代：无」、没有「仍待处理」', async () => {
    const { user } = await openPanel({
      desired: [desiredBody({ id: NEWEST })],
    });
    expect(queueCell()).not.toHaveTextContent('最新');
    expect(queueCell()).not.toHaveTextContent('较早');
    await user.click(picks()[0]);
    await user.click(publishButton());
    expect(preview()).toHaveTextContent('无：同一对象没有比它早的未发布草案');
    expect(preview()).not.toHaveTextContent('仍待处理');
    expect(
      within(preview()).getByRole('button', { name: '确认发布' }),
    ).toBeInTheDocument();
  });

  it('服务端回 409 newer_desired_same_submitter（读列表之后同一提交者又交了更新的）：说清没发布、挡住它的是哪份', async () => {
    const { user, server } = await openPanel({
      desired: [desiredBody({ id: OLDER })],
      write: (path) => {
        if (path === '/api/v1/policy/activate')
          throw new ApiError('同一提交者还有更新的未发布 desired', 409, {
            code: 'newer_desired_same_submitter',
            error: '同一提交者还有更新的未发布草案',
            newer_desired_ids: [NEWEST],
          });
        throw new ApiError('未知', 404);
      },
    });
    await user.click(picks()[0]);
    await user.click(publishButton());
    await user.click(
      within(preview()).getByRole('button', { name: '确认发布' }),
    );
    // 校验报告里的扩大项提示也是 alert，按文字找发布失败那一句。
    expect(
      await within(queueCell()).findByText(
        `没有发布：同一提交者还有更新的未发布草案（${NEWEST.slice(0, 12)}），只能发布其中最新的那一份；要发这份，先撤回更新的。`,
      ),
    ).toHaveAttribute('role', 'alert');
    expect(
      server.posts.filter((post) => post.path === '/api/v1/policy/activate'),
    ).toHaveLength(1);
  });

  it('选中的记录每次从新列表取：重读后同一提交者有了更新的草案，就改标较早并挡住发布', async () => {
    const desired = [desiredBody({ id: OLDER })];
    const newer: Record<string, string[]> = {};
    const blocked: Record<string, string[]> = {};
    const { user } = await openPanel({
      desired,
      newer,
      blocked,
      write: (path) => {
        if (path === '/api/v1/policy/validate')
          return { desired: desiredBody({ id: OLDER }) };
        throw new ApiError('未知', 404);
      },
    });
    await user.click(picks()[0]);
    expect(publishButton()).toBeEnabled();
    // 别处又交了一份同一提交者的更新草案：下一次重读就看得到。
    desired.unshift(desiredBody({ id: NEWEST }));
    newer[OLDER] = [NEWEST];
    blocked[OLDER] = [NEWEST];
    await user.click(screen.getByRole('button', { name: '重新校验' }));
    expect(
      await within(queueCell()).findByText('已重新校验：已校验，可发布。'),
    ).toBeVisible();
    expect(publishButton()).toBeDisabled();
    expect(
      screen.getByText(/同一提交者还有更新的未发布草案（01JZ00000BBB）/),
    ).toBeInTheDocument();
  });
});
