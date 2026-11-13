import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { QueueReview } from '../../cockpit/InlineReview';
import { QueueCell } from '../../cockpit/QueueCell';
import { createLaterStore } from '../../cockpit/later';
import type { OverviewState } from '../../cockpit/model';
import { planPending } from '../../cockpit/pending';
import { publishKey, reviewOrigin, withdrawKey } from '../../cockpit/review';
import { overviewLabels } from '../../collab/executorGrouping';
import { createSingleKeysPreference } from '../../keys/preference';
import { publishPreview, withdrawPreview } from '../../policy/previews';
import type { RecentAuth } from '../../policy/recentAuth';
import type {
  Activation,
  OpenPreview,
  PolicyActions,
} from '../../policy/usePolicyActions';
import type { RecentAuthRead } from '../../policy/useRecentAuth';
import { policyStatus } from '../../policyModel';
import type { ListedDesired } from '../../policyModel';
import type { Action } from '../../types';
import { KeysHarness } from '../keys/KeysHarness';
import { NOW, fixtureData, policyReady, read } from './cockpitFixtures';

/**
 * 队列这一格接上就地审批之后的那几件事（T-200），用替身的写动作和近期认证读取摆出各种状态：
 * 结果写在哪、倒计时什么时候画、那一行不见之后选谁、焦点交给谁。整条路走真的那一份在 inlineReview.test.tsx。
 */

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

const data = fixtureData();
const READY: OverviewState = { kind: 'ready', overview: data.overview };
const STATUS = policyStatus(read(data.api, '/api/v1/policy'));
/** 假数据那份 AI 提案（能力集 claude，09:12 交的）。 */
const [PROPOSAL] = data.desired;
const minutesAfter = (minutes: number) =>
  new Date(Date.parse(PROPOSAL.submitted_at) + minutes * 60_000).toISOString();
/** 主人自己交的两份草案，排在提案后面。 */
const own = (id: string, subject: string, minutes: number): ListedDesired => ({
  ...PROPOSAL,
  id,
  subject,
  submitter_kind: 'owner',
  agent_type: null,
  agent_instance: null,
  status: 'validated',
  submitted_at: minutesAfter(minutes),
});
const CODEX = own('01M33ZA2G0HQ8K8HMTV4NH3KK1', 'codex', 1);
const GEMINI = own('01M33ZA2G0HQ8K8HMTV4NH3KK2', 'gemini', 2);
const TITLE = {
  claude: '能力集 claude',
  codex: '能力集 codex',
  gemini: '能力集 gemini',
};

function stubActions(overrides: Partial<PolicyActions> = {}): PolicyActions {
  return {
    busy: false,
    notice: null,
    error: null,
    pending: null,
    preview: null,
    run: vi.fn(async () => {}),
    submit: vi.fn(),
    amend: vi.fn(),
    validate: vi.fn(),
    withdraw: vi.fn(),
    togglePreview: vi.fn(),
    previewRevoke: vi.fn(),
    previewWithdraw: vi.fn(),
    confirmPreview: vi.fn(),
    closePreview: vi.fn(),
    closePreviewIf: vi.fn(),
    retryAfterStepUp: vi.fn(),
    closeStepUp: vi.fn(),
    ...overrides,
  };
}

function stubAuth(
  state: RecentAuth = { kind: 'unknown' },
  now: () => number = () => 0,
): RecentAuthRead {
  return {
    state,
    read: vi.fn(async () => state),
    expire: vi.fn(),
    now,
  };
}

const withdrawOpen = (desired: ListedDesired): OpenPreview => ({
  key: withdrawKey(desired.id),
  kind: 'withdraw',
  origin: reviewOrigin(desired.id),
  record: desired,
  done: '已退回。',
  content: withdrawPreview(desired),
});

const activationFor = (desired: ListedDesired): Activation => ({
  request: {
    desired_id: desired.id,
    expected_effective_hash: desired.report!.base_effective_hash,
  },
  label: TITLE.claude,
  action: `发布 ${TITLE.claude}`,
  origin: reviewOrigin(desired.id),
  supersedes: 0,
  remains: 0,
});

const publishOpen = (desired: ListedDesired): OpenPreview => ({
  key: publishKey(desired.id),
  kind: 'activate',
  activation: activationFor(desired),
  content: publishPreview(desired, desired.report!, STATUS, [], []),
});

interface Scene {
  drafts: readonly ListedDesired[];
  actions?: PolicyActions;
  auth?: RecentAuthRead;
  inbox?: readonly Action[];
}

/** 渲染队列这一格（外面包着全站按键监听，旁边放一个队列之外的按钮）；`rerender` 换一幕。 */
function renderQueue(scene: Scene) {
  const later = createLaterStore({ storage: memory(), now: () => Date.now() });
  const preference = createSingleKeysPreference(memory());
  const handlers = {
    onOpen: vi.fn(),
    onOpenTask: vi.fn(),
    onOpenPolicy: vi.fn(),
    onRetryPolicy: vi.fn(),
  };
  const element = (next: Scene) => {
    const policy = policyReady(next.drafts);
    const review: QueueReview = {
      actions: next.actions ?? stubActions(),
      auth: next.auth ?? stubAuth(),
    };
    return (
      <KeysHarness preference={preference}>
        <button type="button">队列外面的按钮</button>
        <QueueCell
          plan={planPending({
            board: READY,
            policy,
            inbox: next.inbox ?? data.workbench.inbox,
            projects: data.workbench.projects,
          })}
          board={READY}
          policy={policy}
          labels={overviewLabels(data.overview)}
          asOf="20:31"
          later={later}
          review={review}
          {...handlers}
        />
      </KeysHarness>
    );
  };
  const view = render(element(scene));
  return { rerender: (next: Scene) => view.rerender(element(next)) };
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
const outcome = () =>
  queue().querySelector<HTMLElement>('.cockpit-queue__outcome');
const countdown = () =>
  queue().querySelector<HTMLElement>('.cockpit-queue__auth');
const approve = () =>
  queue().querySelector<HTMLButtonElement>('[data-queue-approve]')!;
const reject = () =>
  queue().querySelector<HTMLButtonElement>('[data-queue-reject]')!;
const validateButton = () =>
  queue().querySelector<HTMLButtonElement>('[data-review-validate]');
const previewBox = () => screen.queryByRole('group', { name: /^执行预览/ });
const outside = () => screen.getByRole('button', { name: '队列外面的按钮' });
const press = (key: string) =>
  fireEvent.keyDown(document.activeElement ?? document.body, { key });

beforeEach(() => {
  vi.spyOn(Date, 'now').mockReturnValue(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('就地审批的结果写在哪', () => {
  const origin = reviewOrigin(PROPOSAL.id);
  const DONE = `已退回 ${TITLE.claude}（草案 ${PROPOSAL.id.slice(0, 12)}）。`;

  it('那一行已经不在队列里：写在队列标题下面，是播报区', () => {
    renderQueue({
      drafts: [CODEX],
      actions: stubActions({ notice: { origin, text: DONE } }),
    });
    expect(outcome()).toHaveTextContent(DONE);
    expect(outcome()).toHaveAttribute('role', 'status');
  });

  it('那一行还选中着：结果就在它自己的就地审批里，标题下面不重复', () => {
    renderQueue({
      drafts: [PROPOSAL, CODEX],
      actions: stubActions({ notice: { origin, text: '已重新校验。' } }),
    });
    expect(outcome()).toBeNull();
    expect(within(selectedRow()).getByText('已重新校验。')).toHaveAttribute(
      'role',
      'status',
    );
  });

  it('那一行还在、但选中的是别的行：写在标题下面', () => {
    renderQueue({
      drafts: [PROPOSAL, CODEX],
      actions: stubActions({ notice: { origin, text: '已重新校验。' } }),
    });
    fireEvent.click(select(1));
    expect(outcome()).toHaveTextContent('已重新校验。');
  });

  it('不是就地审批发起的反馈：不归这里写', () => {
    renderQueue({
      drafts: [CODEX],
      actions: stubActions({
        notice: { origin: 'queue', text: '别处的提示' },
        error: { origin: 'editor', text: '别处的报错' },
      }),
    });
    expect(outcome()).toBeNull();
  });

  it('出错而那一行不见了：标题下面报错（alert），点明是刚才动的哪一份', () => {
    const confirmPreview = vi.fn();
    const { rerender } = renderQueue({
      drafts: [PROPOSAL, CODEX],
      actions: stubActions({
        preview: withdrawOpen(PROPOSAL),
        confirmPreview,
      }),
    });
    fireEvent.click(
      within(previewBox()!).getByRole('button', { name: '确认退回' }),
    );
    expect(confirmPreview).toHaveBeenCalledTimes(1);
    rerender({
      drafts: [CODEX],
      actions: stubActions({ error: { origin, text: '撤回失败。' } }),
    });
    expect(outcome()).toHaveAttribute('role', 'alert');
    expect(outcome()).toHaveTextContent(`${TITLE.claude}：撤回失败。`);
  });

  it('出错的那一份不是在这里动的：原话照写，不硬安一个名字', () => {
    renderQueue({
      drafts: [CODEX],
      actions: stubActions({ error: { origin, text: '撤回失败。' } }),
    });
    expect(outcome()).toHaveAttribute('role', 'alert');
    expect(outcome()!.textContent).toBe('撤回失败。');
  });

  it('报错和提示同时有：先写报错', () => {
    renderQueue({
      drafts: [CODEX],
      actions: stubActions({
        notice: { origin, text: DONE },
        error: { origin, text: '撤回失败。' },
      }),
    });
    expect(outcome()).toHaveTextContent('撤回失败。');
    expect(outcome()).not.toHaveTextContent(DONE);
  });
});

describe('「二次认证还有效」那一行什么时候画', () => {
  const VALID: RecentAuth = {
    kind: 'valid',
    window: { remainingMs: 65_000, at: 0 },
  };

  it('知道还有效、队列里还有策略草案：写在标题下面，照给的时钟算', () => {
    renderQueue({ drafts: [PROPOSAL], auth: stubAuth(VALID, () => 5_000) });
    expect(countdown()).toHaveTextContent('二次认证还有效 1 分：');
    expect(countdown()).toHaveClass('cockpit-note');
  });

  it('队列里没有策略草案了：不写——没有可批的，这个数对人没用', () => {
    renderQueue({ drafts: [], auth: stubAuth(VALID) });
    expect(countdown()).toBeNull();
  });

  it('不知道、没有：不写', () => {
    const { rerender } = renderQueue({ drafts: [PROPOSAL] });
    expect(countdown()).toBeNull();
    rerender({ drafts: [PROPOSAL], auth: stubAuth({ kind: 'none' }) });
    expect(countdown()).toBeNull();
    rerender({ drafts: [PROPOSAL], auth: stubAuth({ kind: 'no-path' }) });
    expect(countdown()).toBeNull();
  });

  it('走到头：告诉近期认证的那一份读取（它把「还有效」改成「没有」）', () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    let clock = 0;
    const auth = stubAuth(VALID, () => clock);
    renderQueue({ drafts: [PROPOSAL], auth });
    expect(countdown()).toHaveTextContent('1 分 5 秒');
    clock = 64_000;
    act(() => {
      vi.advanceTimersByTime(64_000);
    });
    expect(countdown()).toHaveTextContent('还有效 1 秒');
    expect(auth.expire).not.toHaveBeenCalled();
    clock = 65_000;
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(countdown()).toBeNull();
    expect(auth.expire).toHaveBeenCalledTimes(1);
  });
});

describe('这份草案在这里批不了', () => {
  it('还没有校验报告：写明先校验，「批准并发布」不可点，给一个「校验」', () => {
    const bare: ListedDesired = {
      ...PROPOSAL,
      report: null,
      validated_at: null,
    };
    const validate = vi.fn<PolicyActions['validate']>();
    const auth = stubAuth();
    renderQueue({ drafts: [bare], actions: stubActions({ validate }), auth });
    expect(
      within(selectedRow()).getByText('这份草案还没有校验报告：先校验。'),
    ).toBeInTheDocument();
    expect(approve()).toBeDisabled();
    expect(reject()).toBeEnabled();
    expect(validateButton()).toHaveTextContent('校验');
    expect(validateButton()).toHaveAccessibleName(`${TITLE.claude}，校验`);
    // A 键替人点按钮：按钮不可点，键也没反应。
    press('a');
    expect(auth.read).not.toHaveBeenCalled();

    fireEvent.click(validateButton()!);
    expect(validate).toHaveBeenCalledTimes(1);
    const [origin, record, describeResult] = validate.mock.calls[0];
    expect(origin).toBe(reviewOrigin(bare.id));
    expect(record).toBe(bare);
    // 校验之后说的那一句由这里给：点明是哪一份；没通过还写明它去了哪里。
    const which = `${TITLE.claude}（草案 ${bare.id.slice(0, 12)}）`;
    expect(describeResult?.({ ...bare, status: 'validated' })).toBe(
      `已重新校验 ${which}：已校验，可发布。`,
    );
    expect(describeResult?.({ ...bare, status: 'rejected' })).toContain(
      '校验未通过，它不再排在这里等你点头',
    );
  });

  it('校验没通过：写明去策略页看问题或者在这里退回；只有退回可点，没有「重新校验」', () => {
    const failed: ListedDesired = {
      ...PROPOSAL,
      report: { ...PROPOSAL.report!, ok: false },
    };
    renderQueue({ drafts: [failed] });
    expect(
      within(selectedRow()).getByText(
        '校验没通过：去策略页看逐条问题，改好重交，或者在这里退回。',
      ),
    ).toBeInTheDocument();
    expect(approve()).toBeDisabled();
    expect(reject()).toBeEnabled();
    expect(validateButton()).toBeNull();
  });

  it('有一件写动作在途：三个按钮都不可点，A、R 没反应', () => {
    const previewWithdraw = vi.fn();
    const auth = stubAuth();
    renderQueue({
      drafts: [{ ...PROPOSAL, report: null, validated_at: null }, CODEX],
      actions: stubActions({ busy: true, previewWithdraw }),
      auth,
    });
    expect(approve()).toBeDisabled();
    expect(reject()).toBeDisabled();
    expect(validateButton()).toBeDisabled();
    fireEvent.click(select(1));
    expect(approve()).toBeDisabled();
    press('a');
    press('r');
    expect(auth.read).not.toHaveBeenCalled();
    expect(previewWithdraw).not.toHaveBeenCalled();
  });

  it('R 先把焦点落到「退回」上再点：和用鼠标点它一样，确认框收起时焦点回得到这个按钮', () => {
    const previewWithdraw = vi.fn<PolicyActions['previewWithdraw']>();
    renderQueue({
      drafts: [PROPOSAL],
      actions: stubActions({ previewWithdraw }),
    });
    outside().focus();
    press('r');
    expect(reject()).toHaveFocus();
    expect(previewWithdraw).toHaveBeenCalledTimes(1);
    const [key, origin, record] = previewWithdraw.mock.calls[0];
    expect(key).toBe(withdrawKey(PROPOSAL.id));
    expect(origin).toBe(reviewOrigin(PROPOSAL.id));
    expect(record).toBe(PROPOSAL);
  });

  it('按钮不可点：键没反应，焦点也不动', () => {
    renderQueue({
      drafts: [PROPOSAL],
      actions: stubActions({ busy: true }),
    });
    outside().focus();
    press('a');
    press('r');
    expect(outside()).toHaveFocus();
  });

  it('认证窗口开着（有一件在等验证码）：批准、退回都不再起新的一件', () => {
    const previewWithdraw = vi.fn();
    const togglePreview = vi.fn();
    const auth = stubAuth();
    renderQueue({
      drafts: [PROPOSAL],
      actions: stubActions({
        pending: { ...activationFor(PROPOSAL), attempt: 0 },
        previewWithdraw,
        togglePreview,
      }),
      auth,
    });
    fireEvent.click(approve());
    fireEvent.click(reject());
    // 窗口开着时不读因子状态：会话那一层每读一次就作废窗口里那一份快照。
    expect(auth.read).not.toHaveBeenCalled();
    expect(togglePreview).not.toHaveBeenCalled();
    expect(previewWithdraw).not.toHaveBeenCalled();
  });
});

describe('执行预览画在哪一行，细节列多少', () => {
  /** 八条扩大项：超过就地细节平时列的六条。 */
  const wide: ListedDesired = {
    ...PROPOSAL,
    report: {
      ...PROPOSAL.report!,
      expansions: Array.from({ length: 8 }, (_, index) => ({
        path: 'read.project_whitelist',
        before: null,
        after: `项目/新增${index + 1}`,
        note: '读取范围变宽',
      })),
    },
  };
  const expansions = () =>
    within(
      within(selectedRow().querySelector<HTMLElement>('.cockpit-detail')!)
        .getByRole('heading', { level: 3, name: /^扩大/ })
        .closest('section')!,
    ).getAllByRole('listitem');

  it('没开预览：每段最多六条，写明还有几条', () => {
    renderQueue({ drafts: [wide] });
    expect(expansions()).toHaveLength(6);
    expect(selectedRow()).toHaveTextContent('还有 2 条没列出来。');
    expect(previewBox()).toBeNull();
  });

  it('批准的预览开着：这一行的细节全部列出——要点头的东西得先看全', () => {
    renderQueue({
      drafts: [wide],
      actions: stubActions({ preview: publishOpen(wide) }),
    });
    expect(selectedRow()).toContainElement(previewBox());
    expect(expansions()).toHaveLength(8);
    expect(selectedRow()).not.toHaveTextContent('没列出来');
    expect(approve()).toHaveAttribute('aria-expanded', 'true');
    expect(reject()).toHaveAttribute('aria-expanded', 'false');
  });

  it('退回的确认框开着：细节照旧六条（退回不看扩大项）', () => {
    renderQueue({
      drafts: [wide],
      actions: stubActions({ preview: withdrawOpen(wide) }),
    });
    expect(selectedRow()).toContainElement(previewBox());
    expect(expansions()).toHaveLength(6);
    expect(reject()).toHaveAttribute('aria-expanded', 'true');
    expect(approve()).toHaveAttribute('aria-expanded', 'false');
  });

  it('预览是别的草案的：不画在这一行', () => {
    renderQueue({
      drafts: [wide, CODEX],
      actions: stubActions({ preview: publishOpen(CODEX) }),
    });
    expect(previewBox()).toBeNull();
    expect(expansions()).toHaveLength(6);
  });

  it('预览框里的「确认」「取消」各调一次，不直接发请求', () => {
    const confirmPreview = vi.fn();
    const closePreview = vi.fn();
    renderQueue({
      drafts: [wide],
      actions: stubActions({
        preview: publishOpen(wide),
        confirmPreview,
        closePreview,
      }),
    });
    // 替身直接摆出一个开着的预览，不是这一行自己打开的：挂上来时组件会先要求收一次。那一次不算。
    closePreview.mockClear();
    fireEvent.click(
      within(previewBox()!).getByRole('button', { name: '取消' }),
    );
    expect(closePreview).toHaveBeenCalledTimes(1);
    expect(confirmPreview).not.toHaveBeenCalled();
    fireEvent.click(
      within(previewBox()!).getByRole('button', { name: '确认发布' }),
    );
    expect(confirmPreview).toHaveBeenCalledTimes(1);
  });

  it('人走开了（换了一行）：只收这一行自己的预览', () => {
    const closePreviewIf = vi.fn<PolicyActions['closePreviewIf']>();
    renderQueue({
      drafts: [PROPOSAL, CODEX],
      actions: stubActions({ closePreviewIf }),
    });
    press('j');
    expect(closePreviewIf).toHaveBeenCalledTimes(1);
    const [matches] = closePreviewIf.mock.calls[0];
    expect(matches(publishOpen(PROPOSAL))).toBe(true);
    expect(matches(withdrawOpen(PROPOSAL))).toBe(true);
    expect(matches(publishOpen(CODEX))).toBe(false);
    expect(matches(withdrawOpen(CODEX))).toBe(false);
  });
});

describe('选中的那一行不在了：改选顶上来的那一行', () => {
  it('点过的那一行被处理掉：选中留在原来的位置上，不跳回第一行', () => {
    const { rerender } = renderQueue({ drafts: [PROPOSAL, CODEX, GEMINI] });
    expect(titles().slice(0, 3)).toEqual([
      TITLE.claude,
      TITLE.codex,
      TITLE.gemini,
    ]);
    fireEvent.click(select(1));
    expect(selectedIndex()).toBe(1);
    rerender({ drafts: [PROPOSAL, GEMINI] });
    expect(selectedIndex()).toBe(1);
    expect(titles()[1]).toBe(TITLE.gemini);
  });

  it('不在的是最后一行：选它前面那一行', () => {
    const inbox = data.workbench.inbox;
    const { rerender } = renderQueue({ drafts: [PROPOSAL] });
    const last = rows().length - 1;
    fireEvent.click(select(last));
    expect(selectedIndex()).toBe(last);
    rerender({ drafts: [PROPOSAL], inbox: inbox.slice(0, -1) });
    expect(rows()).toHaveLength(last);
    expect(selectedIndex()).toBe(last - 1);
  });

  it('没点过（缺省选第一行）、第一行被处理掉：还是第一行', () => {
    const { rerender } = renderQueue({ drafts: [PROPOSAL, CODEX] });
    expect(selectedIndex()).toBe(0);
    rerender({ drafts: [CODEX] });
    expect(selectedIndex()).toBe(0);
    expect(titles()[0]).toBe(TITLE.codex);
  });

  it('选中的那一行还在、别的行少了：选中的还是它', () => {
    const { rerender } = renderQueue({ drafts: [PROPOSAL, CODEX, GEMINI] });
    fireEvent.click(select(2));
    rerender({ drafts: [PROPOSAL, GEMINI] });
    expect(selectedIndex()).toBe(1);
    expect(titles()[selectedIndex()]).toBe(TITLE.gemini);
  });

  it('收起了（没有选中的行）：行少了也还是收起', () => {
    const { rerender } = renderQueue({ drafts: [PROPOSAL, CODEX] });
    fireEvent.click(select(0));
    expect(selectedIndex()).toBe(-1);
    rerender({ drafts: [CODEX] });
    expect(selectedIndex()).toBe(-1);
  });

  it('连着少两行：每次都落在顶上来的那一行', () => {
    const { rerender } = renderQueue({ drafts: [PROPOSAL, CODEX, GEMINI] });
    fireEvent.click(select(1));
    rerender({ drafts: [PROPOSAL, GEMINI] });
    expect(titles()[selectedIndex()]).toBe(TITLE.gemini);
    rerender({ drafts: [PROPOSAL] });
    // 顶上来的是黑板里等你的第一件。
    expect(selectedIndex()).toBe(1);
    expect(selectedRow()).toHaveAttribute('data-source', 'board');
  });
});

describe('做完之后焦点交给谁', () => {
  /**
   * 展开一个确认框并点确认：这一行记下「刚交出去一件」。真的那一份写动作在确认的同一下点击里就进入
   * 「在途」（policy/usePolicyActions 的 run 同步置位），替身照样摆——不然这一件当场就算做完了
   * （那一行还在，不交），后面每一条都成了空过。
   */
  function confirmIn(preview: OpenPreview, name: string) {
    const view: ReturnType<typeof renderQueue> = renderQueue({
      drafts: [PROPOSAL, CODEX],
      actions: stubActions({
        preview,
        confirmPreview: () =>
          view.rerender({
            drafts: [PROPOSAL, CODEX],
            actions: stubActions({ busy: true }),
          }),
      }),
    });
    fireEvent.click(within(previewBox()!).getByRole('button', { name }));
    // 在途：确认框收起了，按钮都不可点，焦点还没动。
    expect(previewBox()).toBeNull();
    expect(approve()).toBeDisabled();
    expect(reject()).toBeDisabled();
    expect(primary()).not.toHaveFocus();
    return view;
  }
  const confirmWithdraw = () => confirmIn(withdrawOpen(PROPOSAL), '确认退回');

  it('那一行不见了：焦点交给新选中那一行的主按钮，接着审下一件', () => {
    const { rerender } = confirmWithdraw();
    rerender({ drafts: [CODEX] });
    expect(titles()[0]).toBe(TITLE.codex);
    expect(selectedIndex()).toBe(0);
    expect(primary()).toHaveFocus();
  });

  it('列表先变、动作后结束：等动作结束才交', () => {
    const { rerender } = confirmWithdraw();
    rerender({ drafts: [CODEX], actions: stubActions({ busy: true }) });
    expect(primary()).not.toHaveFocus();
    rerender({ drafts: [CODEX] });
    expect(primary()).toHaveFocus();
  });

  it('被要求认证、窗口还开着：不交；认证后重试做成了、那一行不见了才交', () => {
    const { rerender } = confirmIn(publishOpen(PROPOSAL), '确认发布');
    rerender({
      drafts: [PROPOSAL, CODEX],
      actions: stubActions({
        pending: { ...activationFor(PROPOSAL), attempt: 0 },
      }),
    });
    expect(primary()).not.toHaveFocus();
    // 认证通过，原样重试：又在途。
    rerender({
      drafts: [PROPOSAL, CODEX],
      actions: stubActions({ busy: true }),
    });
    expect(primary()).not.toHaveFocus();
    rerender({ drafts: [CODEX] });
    expect(primary()).toHaveFocus();
  });

  it('那一行还在（没做成）：不交，之后别的原因让它不见了也不再交', () => {
    const { rerender } = confirmWithdraw();
    rerender({
      drafts: [PROPOSAL, CODEX],
      actions: stubActions({
        error: { origin: reviewOrigin(PROPOSAL.id), text: '撤回失败。' },
      }),
    });
    expect(primary()).not.toHaveFocus();
    rerender({ drafts: [CODEX] });
    expect(primary()).not.toHaveFocus();
  });

  it('被要求认证、人关掉了窗口（没做成）：不交', () => {
    const { rerender } = confirmIn(publishOpen(PROPOSAL), '确认发布');
    rerender({
      drafts: [PROPOSAL, CODEX],
      actions: stubActions({
        pending: { ...activationFor(PROPOSAL), attempt: 0 },
      }),
    });
    rerender({ drafts: [PROPOSAL, CODEX] });
    expect(primary()).not.toHaveFocus();
    rerender({ drafts: [CODEX] });
    expect(primary()).not.toHaveFocus();
  });

  it('焦点落在这一格里别的地方（另一行的标题上）：照样交给主按钮', () => {
    const { rerender } = confirmWithdraw();
    select(1).focus();
    rerender({ drafts: [CODEX] });
    expect(primary()).toHaveFocus();
  });

  it('人已经把焦点放到队列外面：不抢', () => {
    const { rerender } = confirmWithdraw();
    outside().focus();
    rerender({ drafts: [CODEX] });
    expect(outside()).toHaveFocus();
  });

  it('没在这里动过手，那一行自己不见了（别处处理掉）：只改选，不动焦点', () => {
    const { rerender } = renderQueue({ drafts: [PROPOSAL, CODEX] });
    rerender({ drafts: [CODEX] });
    expect(selectedIndex()).toBe(0);
    expect(document.body).toHaveFocus();
  });
});

describe('键位提示', () => {
  const hints = () =>
    within(within(queue()).getByRole('list', { name: '这一格的键盘快捷键' }))
      .getAllByRole('listitem')
      .map((item) => item.textContent);

  it('选中的是策略草案：多写 A 批准、R 退回；选中别的行不写', () => {
    renderQueue({ drafts: [PROPOSAL] });
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

  it('选中的是自己交的草案：R 写「撤回」，和按钮上的叫法一样', () => {
    renderQueue({ drafts: [CODEX] });
    expect(reject()).toHaveTextContent('撤回');
    expect(hints()).toEqual([
      'JK上下移动',
      '↵去处理',
      'H稍后',
      'A批准',
      'R撤回',
      'G跳到别的视图',
    ]);
  });
});
