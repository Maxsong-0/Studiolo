import {
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useReducer,
  useRef,
  useState,
  useSyncExternalStore,
  type AnimationEvent,
  type ReactNode,
} from 'react';
import { QUEUE_KEYS } from '../keys/shortcuts';
import { KeyScopeContext, useKeyBindings } from '../keys/useGlobalKeys';
import { AuthCountdown } from '../policy/AuthCountdown';
import { withdrawVerb } from '../policy/previews';
import type { Feedback } from '../policy/usePolicyActions';
import type { ListedDesired } from '../policyModel';
import type { OpenSource } from '../types';
import { Button, KeyHint } from '../ui';
import { cssVars } from '../ui/dom';
import { EntryDetail } from './EntryDetail';
import { InlineReview, type QueueReview } from './InlineReview';
import { laterStore, splitLater, type LaterStore } from './later';
import type { OverviewState, PolicyPendingState } from './model';
import {
  entryKey,
  policyEntryKey,
  type PendingEntry,
  type PendingPlan,
} from './pending';
import { queueText } from './queueText';
import { REVIEW_ORIGIN, publishKey } from './review';
import { WAIT_MARKS, waitFraction, waitLabel } from './wait';
import '../styles/cockpit.css';
import '../styles/keys.css';

/** 「稍后」的行淡出之后才挪走；动画没跑起来（样式没载入、行被换掉）时最多等这么久。 */
const LEAVE_FALLBACK_MS = 400;

/** 策略还没读到时的空草案列表：同一个数组，不让下面的行白白重画。 */
const NO_DRAFTS: readonly ListedDesired[] = [];

/** 就地审批里刚交出去的那一件写动作：哪一行、叫什么（出错时写在队列标题下面，要点明是哪一份）。 */
interface Acted {
  key: string;
  label: string;
}

/**
 * 就地审批的结果要不要写在队列标题下面：结果属于的那一行不是眼下选中的（多半是发布、退回之后
 * 已经不在队列里了）才写；还选中着，结果就在那一行自己的就地审批里。
 */
function reviewOutcome(
  review: QueueReview,
  selected: string | null,
  acted: Acted | null,
): { text: string; failed: boolean } | null {
  const elsewhere = (feedback: Feedback | null) => {
    if (!feedback?.origin.startsWith(REVIEW_ORIGIN)) return null;
    const key = policyEntryKey(feedback.origin.slice(REVIEW_ORIGIN.length));
    return key === selected ? null : { key, text: feedback.text };
  };
  const failed = elsewhere(review.actions.error);
  if (failed)
    return {
      failed: true,
      text:
        acted?.key === failed.key
          ? `${acted.label}：${failed.text}`
          : failed.text,
    };
  const done = elsewhere(review.actions.notice);
  return done ? { failed: false, text: done.text } : null;
}

/**
 * 「等了多久」的图例：一条刻度，三道线是 10 分、1 时、1 天。
 * 10 分那道只画线不写字：它离 1 时只有两成宽，三个字写上去会和「1时」叠在一起。
 */
function WaitLegend() {
  return (
    <div className="cockpit-queue__legend">
      <span>等了多久</span>
      <span className="cockpit-scale" aria-hidden="true">
        {WAIT_MARKS.map((mark, index) => (
          <span
            key={mark.label}
            className="cockpit-scale__mark"
            style={cssVars({ '--cockpit-at': `${mark.at * 100}%` })}
          >
            {index > 0 ? (
              <span className="cockpit-scale__label">{mark.label}</span>
            ) : null}
          </span>
        ))}
      </span>
      <span className="sr-only">
        刻度条按时长的对数画：10 分在三成处，1 小时在一半处，1 天在九成处。
      </span>
    </div>
  );
}

/** 一行的等待时长：一行等宽文字，下面一条对数刻度条；来源不带时间时写「—」，不画条。 */
function Waited({ ms }: { ms: number | null }) {
  if (ms === null)
    return (
      <span className="cockpit-queue__waited" title="这一件的来源不带时间">
        —<span className="sr-only">，不知道等了多久</span>
      </span>
    );
  return (
    <>
      <span className="cockpit-queue__waited">
        <span className="sr-only">已等 </span>
        {waitLabel(ms)}
      </span>
      <span
        className="cockpit-wait"
        aria-hidden="true"
        style={cssVars({
          '--cockpit-wait': `${(waitFraction(ms) * 100).toFixed(1)}%`,
        })}
      >
        <span className="cockpit-wait__fill" />
      </span>
    </>
  );
}

function Row({
  entry,
  selected,
  later,
  leaving,
  labels,
  review,
  drafts,
  onToggle,
  onOpen,
  onLater,
  onLeft,
  onReviewed,
}: {
  entry: PendingEntry;
  selected: boolean;
  /** 这一行在「稍后再看」里。 */
  later: boolean;
  /** 刚按了「稍后」，正在淡出。 */
  leaving: boolean;
  labels: ReadonlyMap<string, string> | undefined;
  /** 就地审批（只对策略草案的行有用）；没给就没有那两个按钮。 */
  review: QueueReview | undefined;
  /** 整份草案列表，给就地审批的执行预览用。 */
  drafts: readonly ListedDesired[];
  onToggle: () => void;
  onOpen: () => void;
  /** 放到「稍后」，或者从「稍后再看」放回队列。 */
  onLater: () => void;
  /** 淡出走完了。 */
  onLeft: () => void;
  /** 就地审批里交出去了一件写动作（确认了发布或退回，或者点了校验）；参数是对象的名字。 */
  onReviewed: (label: string) => void;
}) {
  const detailId = useId();
  const text = queueText(entry, labels);
  // 就地批准的执行预览开着：细节全部列出，不截到前几条。
  const approving =
    entry.source === 'policy' &&
    review?.actions.preview?.key === publishKey(entry.desired.id);
  // 只认这一行自己的淡出：行里别的东西的动画结束也会冒泡上来。
  const left = (event: AnimationEvent<HTMLLIElement>) => {
    if (event.target === event.currentTarget) onLeft();
  };
  return (
    <li
      className="cockpit-queue__row"
      data-source={entry.source}
      data-selected={selected ? 'true' : undefined}
      data-later={later ? 'true' : undefined}
      data-leave={leaving ? 'true' : undefined}
      onAnimationEnd={leaving ? left : undefined}
    >
      <div className="cockpit-queue__index">
        <span className="cockpit-queue__n" aria-hidden="true">
          {String(entry.n).padStart(2, '0')}
        </span>
        <span className="cockpit-queue__kind" title={entry.kind}>
          {entry.kind}
        </span>
      </div>
      <div className="cockpit-queue__body">
        <p className="cockpit-queue__title">
          <button
            type="button"
            className="cockpit-queue__select"
            aria-expanded={selected}
            aria-controls={selected ? detailId : undefined}
            onClick={onToggle}
          >
            {text.title}
          </button>
        </p>
        <p className="cockpit-queue__sub">{text.subtitle}</p>
        {selected ? (
          <div id={detailId} className="cockpit-queue__detail">
            <p className="cockpit-queue__facts">
              <span className="cockpit-queue__facts-label">
                {text.facts.label}
              </span>
              {text.facts.items.map((fact, index) => (
                <span key={index}>{fact}</span>
              ))}
            </p>
            <div className="cockpit-queue__actions">
              <Button
                variant="primary"
                aria-label={text.action.name}
                data-queue-primary=""
                onClick={onOpen}
              >
                {text.action.primary}
              </Button>
              <Button
                variant="text"
                aria-label={`${text.title}，${later ? '放回队列' : '稍后再看'}`}
                onClick={onLater}
              >
                {later ? '放回队列' : '稍后'}
              </Button>
            </div>
            <EntryDetail entry={entry} full={approving} />
            {review && entry.source === 'policy' ? (
              <InlineReview
                desired={entry.desired}
                drafts={drafts}
                actions={review.actions}
                auth={review.auth}
                onActed={onReviewed}
              />
            ) : null}
          </div>
        ) : null}
      </div>
      <div className="cockpit-queue__wait">
        <Waited ms={entry.waitMs} />
      </div>
      <div className="cockpit-queue__open">
        {selected ? null : (
          <button
            type="button"
            className="cockpit-queue__link"
            aria-label={text.action.name}
            onClick={onOpen}
          >
            {text.action.link} <span aria-hidden="true">→</span>
          </button>
        )}
      </div>
    </li>
  );
}

/** 标题下面的一句说明；`key` 是它说的那一路。 */
interface SourceNote {
  key: 'board' | 'policy' | 'truncated';
  body: ReactNode;
}

/**
 * 哪一路还没到手、少了什么：一路一句，放在标题下面。策略读失败时就地给重试。
 * 每句按来源取键，不按位置：前一句消失（那一路读完了）时，后一句还是原来那个节点，
 * 读屏不会把没变的话再念一遍；同一路从「读取中」变成「失败」是同一个节点换字，会念。
 */
function sourceNotes(
  board: OverviewState,
  policy: PolicyPendingState,
  truncated: boolean,
  onRetryPolicy: () => void,
): SourceNote[] {
  const notes: SourceNote[] = [];
  if (board.kind === 'loading')
    notes.push({ key: 'board', body: '正在读取黑板里等你的任务…' });
  if (board.kind === 'error')
    notes.push({
      key: 'board',
      body: '黑板汇总读取失败，等你的黑板任务没列出来；可在右边重试。',
    });
  if (policy.kind === 'loading')
    notes.push({ key: 'policy', body: '正在读取策略里等你的提案和草案…' });
  if (policy.kind === 'error')
    notes.push({
      key: 'policy',
      body: (
        <>
          策略草案读取失败，等你的提案和草案没列出来：{policy.message}
          <Button
            variant="text"
            size="sm"
            className="cockpit-link"
            onClick={onRetryPolicy}
          >
            重试策略草案
          </Button>
        </>
      ),
    });
  if (truncated)
    notes.push({
      key: 'truncated',
      body: '黑板汇总已截断，可能还有等你的任务没列出来。',
    });
  return notes;
}

/** 队列底下的键位提示：只在有鼠标的设备上显示（样式里控制），写的是这一格登记了的那几个键。 */
function QueueKeys({
  enabled,
  rejectVerb,
  onShowAll,
}: {
  /** 单键快捷键开着没有。 */
  enabled: boolean;
  /**
   * 选中的那一行能就地批准、退回（是策略草案）时，R 在这一行叫什么（AI 的提案是「退回」，
   * 自己交的是「撤回」，和按钮上写的一样）：多写 A、R 两个键。别的行是 null，不写。
   */
  rejectVerb: '退回' | '撤回' | null;
  onShowAll: () => void;
}) {
  return (
    <div className="cockpit-queue__keys">
      {enabled ? (
        <ul aria-label="这一格的键盘快捷键">
          <li>
            <KeyHint className="keys-cap" keys={['J']} />
            <KeyHint className="keys-cap" keys={['K']} />
            上下移动
          </li>
          <li>
            <KeyHint className="keys-cap" keys={['↵']} />
            去处理
          </li>
          <li>
            <KeyHint className="keys-cap" keys={['H']} />
            稍后
          </li>
          {rejectVerb ? (
            <>
              <li>
                <KeyHint className="keys-cap" keys={['A']} />
                批准
              </li>
              <li>
                <KeyHint className="keys-cap" keys={['R']} />
                {rejectVerb}
              </li>
            </>
          ) : null}
          <li>
            <KeyHint className="keys-cap" keys={['G']} />
            跳到别的视图
          </li>
        </ul>
      ) : (
        <span>单键快捷键已关闭</span>
      )}
      <Button
        variant="text"
        size="sm"
        className="cockpit-link"
        onClick={onShowAll}
      >
        全部快捷键
        {enabled ? <KeyHint className="keys-cap" keys={['?']} /> : null}
      </Button>
    </div>
  );
}

/**
 * 左上：等你处理的事（T-195、T-197；设计规范第 6、16 节）。
 * 巨号和「件事等你处理」同一行，下面是一列队列：序号与类型、标题与出处、等了多久、去处理。
 * 选中的那一行多一行事实、一个实心主按钮（全页唯一的黑色实心块）、「稍后」，和就地展开的细节；
 * 其余行行尾是文字链接。缺省选中第一行，点哪一行的标题就选中哪一行，再点一次收起。
 * 三路里任何一路没读到，巨号显示「—」，并在标题下说明少了哪一路；已经读到的照常列出。
 *
 * 键盘（T-197）：J／K 上下选（到头不绕回），选中后焦点落到那一行的主按钮上；Enter 去处理；
 * H 把选中的那一件放到队尾的「稍后再看」，再按一次放回。「稍后」只是换个地方排，总数不变。
 *
 * 就地审批（T-200，给了 `review` 才有）：选中的是策略草案时，细节下面有「批准并发布」「退回」，
 * A／R 两个键替人点这两个按钮——都只打开确认那一步。做完之后那一行不在队列里了：结果写在标题下面，
 * 改选顶上来的那一行，焦点交给它的主按钮，接着审下一件。
 */
export function QueueCell({
  plan,
  board,
  policy,
  labels,
  asOf,
  onOpen,
  onOpenTask,
  onOpenPolicy,
  onRetryPolicy,
  later = laterStore(),
  review,
}: {
  plan: PendingPlan;
  board: OverviewState;
  policy: PolicyPendingState;
  /** 整份黑板汇总的执行者显示名；黑板没读到时不给。 */
  labels: ReadonlyMap<string, string> | undefined;
  /** 时长是截至哪一刻算的（本地钟点）；没有带时间的来源时为 null。 */
  asOf: string | null;
  onOpen: OpenSource;
  onOpenTask: (taskId: string) => void;
  /** 去策略页处理这一份草案（草案号）。 */
  onOpenPolicy: (desiredId: string) => void;
  onRetryPolicy: () => void;
  /** 「稍后」的记号；缺省用全站那一份，测试里换成自己的。 */
  later?: LaterStore;
  /** 就地批准、退回策略草案用的那一份写动作和近期认证读取；不给就没有就地审批。 */
  review?: QueueReview;
}) {
  const root = useRef<HTMLElement>(null);
  const scope = useContext(KeyScopeContext);
  const singleKeys = useSyncExternalStore(
    scope.preference.subscribe,
    scope.preference.enabled,
  );
  // undefined＝还没点过，选第一行；null＝点过又收起了。
  const [picked, setPicked] = useState<string | null | undefined>(undefined);
  // 刚按了「稍后」、正在淡出的那一行。
  const [leaving, setLeaving] = useState<string | null>(null);
  // 每加一，就把焦点交给选中行的主按钮（键盘移动、放到稍后、放回之后）。
  const [focusRequest, requestFocus] = useReducer(
    (count: number) => count + 1,
    0,
  );
  // 「稍后」到期时重排一次。
  const [expiryTick, reorder] = useReducer((count: number) => count + 1, 0);
  const marks = useSyncExternalStore(later.subscribe, later.marks);
  const { entries, total } = plan;
  const split = splitLater(entries, marks, later.now());
  // 屏幕上的先后：照常排着的在前，「稍后再看」在后；J／K 按这个顺序走。
  const ordered = [...split.active, ...split.later];
  const keys = ordered.map(entryKey);
  // 点过的那一行在重读之后不在队列里了（已经处理掉）：改选顶上来的那一行（它原来的位置上现在的那一行），
  // 不跳回第一行——连着审的时候不用再找回位置。照上一轮的顺序找它原来排第几。
  const [known, setKnown] = useState(keys);
  if (
    known.length !== keys.length ||
    known.some((key, index) => key !== keys[index])
  ) {
    setKnown(keys);
    if (typeof picked === 'string' && !keys.includes(picked)) {
      const at = known.indexOf(picked);
      setPicked(at === -1 ? undefined : keys[Math.min(at, keys.length - 1)]);
    }
  }
  const selected =
    picked === null
      ? null
      : picked !== undefined && keys.includes(picked)
        ? picked
        : (keys[0] ?? null);
  const drafts = policy.kind === 'ready' ? policy.desired : NO_DRAFTS;
  // 就地审批里刚交出去的那一件写动作。
  const [acted, setActed] = useState<Acted | null>(null);
  // 等它做完、那一行不见了，把焦点交给新选中的那一行；记的是那一行的键。
  const handoff = useRef<string | null>(null);
  const outcome = review ? reviewOutcome(review, selected, acted) : null;
  const selectedEntry = ordered.find(
    (candidate) => entryKey(candidate) === selected,
  );
  // 选中的是策略草案、而且接了就地审批：键位提示多写 A、R，R 用这一行按钮上的叫法。
  const rejectVerb =
    review && selectedEntry?.source === 'policy'
      ? withdrawVerb(selectedEntry.desired)
      : null;
  // 近期二次认证还有效的那一段；队列里还有策略草案时才写——没有可批的，这个数对人没用。
  const authSpan =
    review?.auth.state.kind === 'valid' &&
    entries.some((entry) => entry.source === 'policy')
      ? review.auth.state.window
      : null;
  const notes = sourceNotes(board, policy, plan.truncated, onRetryPolicy);
  const settled =
    board.kind !== 'loading' &&
    board.kind !== 'error' &&
    policy.kind !== 'loading' &&
    policy.kind !== 'error';
  const timed = entries.some((entry) => entry.waitMs !== null);

  // 三路都读完之后才开进场动效：首屏、手动刷新后重新列出来的行不算「新来的」（样式里按 data-live 判）。
  const [live, setLive] = useState(false);
  useEffect(() => setLive(settled), [settled]);

  // 巨号从一个数变成另一个数才滚动；「—」换成数、数换成「—」不滚（首屏、手动刷新）。
  const [shown, setShown] = useState(total);
  const [roll, setRoll] = useState<'up' | 'down' | null>(null);
  if (shown !== total) {
    setShown(total);
    setRoll(
      shown === null || total === null ? null : total > shown ? 'up' : 'down',
    );
  }

  const { nextExpiry } = split;
  useEffect(() => {
    if (nextExpiry === null) return;
    // 多等一点：定时器早到半拍时这一件还没到期，重排了也白排；真早到了，expiryTick 变了会再等一轮。
    const handle = window.setTimeout(
      reorder,
      Math.max(0, nextExpiry - later.now()) + 50,
    );
    return () => window.clearTimeout(handle);
  }, [nextExpiry, later, expiryTick]);

  useLayoutEffect(() => {
    if (focusRequest === 0) return;
    const row = root.current?.querySelector<HTMLElement>(
      '.cockpit-queue__row[data-selected]',
    );
    row
      ?.querySelector<HTMLElement>('[data-queue-primary]')
      ?.focus({ preventScroll: true });
    row?.scrollIntoView({ block: 'nearest' });
  }, [focusRequest]);

  // 就地审批的那一件做完了（没有动作在途、认证窗口也不开着）：那一行不见了（发布了、退回了、校验没通过），
  // 就把焦点交给新选中的那一行，接着审下一件；那一行还在（没做成、取消了认证、校验通过），就不交了。
  // 人已经把焦点放到别处时不抢。
  const settledReview =
    review !== undefined &&
    !review.actions.busy &&
    review.actions.pending === null;
  useLayoutEffect(() => {
    const key = handoff.current;
    if (key === null || !settledReview) return;
    handoff.current = null;
    if (keys.includes(key)) return;
    const active = document.activeElement;
    if (
      active === null ||
      active === document.body ||
      root.current?.contains(active)
    )
      requestFocus();
  });

  function open(entry: PendingEntry) {
    if (entry.source === 'board') onOpenTask(entry.item.id);
    else if (entry.source === 'policy') onOpenPolicy(entry.desired.id);
    else onOpen(entry.action.source);
  }

  /**
   * 把这一件记进「稍后」，返回之后该选中哪一行。`current` 是此刻选中的那一行：
   * 放走的正是它，就改选它后面那一行（没有后面的就选前面的；一行不剩就还选它，它在「稍后再看」里）。
   * 排法按记号现算，不用这一轮重画时的：同一下按键里可能刚落定过另一件。
   */
  function commitLater(key: string, current: string | null): string | null {
    const fresh = splitLater(entries, later.marks(), later.now());
    const activeKeys = fresh.active.map(entryKey);
    const index = activeKeys.indexOf(key);
    // 已经不在照常排着的那一段里了：处理掉了，或者别的标签页已经放过。
    if (index === -1) return current;
    later.later(key, fresh.active[index].since);
    if (current !== key) return current;
    const next = activeKeys[index + 1] ?? activeKeys[index - 1] ?? key;
    setPicked(next);
    return next;
  }

  /** 淡出还没走完就来了下一个键：先把那一件落定，不让人等动画。返回落定之后选中的那一行。 */
  function settle(): string | null {
    if (leaving === null) return selected;
    setLeaving(null);
    return commitLater(leaving, selected);
  }

  function finishLeave(key: string) {
    if (leaving !== key) return;
    setLeaving(null);
    commitLater(key, selected);
    requestFocus();
  }
  // 兜底定时器要调最新的那一份 finishLeave：淡出期间选中行可能已经换了。
  const finish = useRef(finishLeave);
  useLayoutEffect(() => {
    finish.current = finishLeave;
  });
  useEffect(() => {
    if (leaving === null) return;
    const handle = window.setTimeout(
      () => finish.current(leaving),
      LEAVE_FALLBACK_MS,
    );
    return () => window.clearTimeout(handle);
  }, [leaving]);

  function toggleLater(key: string | null) {
    if (key === null) return;
    const fresh = splitLater(entries, later.marks(), later.now());
    if (fresh.later.some((entry) => entryKey(entry) === key)) {
      later.restore(key);
      setPicked(key);
      requestFocus();
    } else if (fresh.active.some((entry) => entryKey(entry) === key)) {
      // 没有偏好查询能力的环境不做动效（和弹层一样保守）：直接挪走。
      if (typeof window.matchMedia === 'function') setLeaving(key);
      else {
        commitLater(key, key);
        requestFocus();
      }
    }
  }

  function move(delta: 1 | -1) {
    const current = settle();
    const fresh = splitLater(entries, later.marks(), later.now());
    const order = [...fresh.active, ...fresh.later].map(entryKey);
    if (order.length === 0) return;
    const index = current === null ? -1 : order.indexOf(current);
    // 没有选中的（收起了）：从第一行开始。到头不绕回。
    const target =
      index === -1 ? 0 : Math.min(order.length - 1, Math.max(0, index + delta));
    setPicked(order[target]);
    requestFocus();
  }

  /**
   * 替人点一下选中行里就地审批的那个按钮（A 批准、R 退回）：键和鼠标走同一条路。
   * 按钮不在（选中的不是策略草案）或者禁用着，就什么都不做；有一行正在淡出时也不做——
   * 那一瞬间屏幕上选中的还是要走的那一行。
   *
   * 焦点先落到按钮上再点，和鼠标点它一样：确认框收起时焦点回到这个按钮（读屏念得出按的是哪一个），
   * 不回到按键之前焦点所在的地方（可能是顶栏、别的格子）；做完之后那一行不见了，焦点才交得到
   * 顶上来的那一行——焦点留在这一格之外的话，队列是不抢的。J／K 也是这样把焦点带进选中行的。
   */
  function press(selector: string) {
    if (leaving !== null) return;
    const button = root.current?.querySelector<HTMLButtonElement>(
      `.cockpit-queue__row[data-selected] ${selector}`,
    );
    if (!button || button.disabled) return;
    button.focus({ preventScroll: true });
    button.click();
  }

  useKeyBindings(
    {
      [QUEUE_KEYS.next]: { run: () => move(1), repeat: true },
      [QUEUE_KEYS.previous]: { run: () => move(-1), repeat: true },
      [QUEUE_KEYS.approve]: { run: () => press('[data-queue-approve]') },
      [QUEUE_KEYS.reject]: { run: () => press('[data-queue-reject]') },
      [QUEUE_KEYS.open]: {
        run: () => {
          const current = settle();
          const entry = ordered.find(
            (candidate) => entryKey(candidate) === current,
          );
          if (entry) open(entry);
        },
        // Enter 在别处另有意思：只在焦点没落在任何东西上、或者落在这一格里时才算「去处理」。
        // 焦点在执行预览的框里时不算：人正在看要确认的东西，一下 Enter 不该把他带去别的页面。
        when: (target) =>
          !(target instanceof Element) ||
          target === document.body ||
          target === document.documentElement ||
          (Boolean(root.current?.contains(target)) &&
            target.closest('.policy-preview') === null),
      },
      [QUEUE_KEYS.later]: { run: () => toggleLater(settle()) },
    },
    ordered.length > 0,
  );

  const row = (entry: PendingEntry, inLater: boolean) => {
    const key = entryKey(entry);
    return (
      <Row
        key={key}
        entry={entry}
        selected={key === selected}
        later={inLater}
        leaving={key === leaving}
        labels={labels}
        review={review}
        drafts={drafts}
        onToggle={() => setPicked(key === selected ? null : key)}
        onOpen={() => open(entry)}
        onLater={() => toggleLater(key)}
        onLeft={() => finishLeave(key)}
        onReviewed={(label) => {
          setActed({ key, label });
          handoff.current = key;
        }}
      />
    );
  };

  return (
    <section ref={root} className="cockpit-queue" aria-label="等你处理">
      <header
        className="cockpit-queue__head"
        style={cssVars({
          // 「—」比一个数字宽，按两位数留位。
          '--cockpit-digits': String(total === null ? 2 : String(total).length),
        })}
      >
        <div
          className="cockpit-queue__count"
          data-state={
            total === null ? 'unknown' : total === 0 ? 'clear' : 'waiting'
          }
          aria-hidden="true"
        >
          {/* 按数取键：数一变就换一个节点，新节点带着 data-roll 滚进来（样式里做），旧的直接撤掉。 */}
          <span
            key={total ?? 'unknown'}
            className="cockpit-queue__count-value"
            data-roll={roll ?? undefined}
          >
            {total ?? '—'}
          </span>
        </div>
        <div className="cockpit-queue__heading">
          <h2>
            <span className="sr-only">
              {total === null ? '不知道有几' : `${total} `}
            </span>
            件事等你处理
          </h2>
          {entries.length > 0 ? (
            <p className="cockpit-queue__meta">
              {plan.kinds.map(({ kind, count }) => (
                <span key={kind}>
                  {kind} {count}
                </span>
              ))}
              {plan.oldestWaitMs !== null ? (
                <span className="cockpit-queue__oldest">
                  最久的已等 {waitLabel(plan.oldestWaitMs)}
                </span>
              ) : null}
            </p>
          ) : null}
          {notes.map((note) => (
            <p key={note.key} className="cockpit-note" role="status">
              {note.body}
            </p>
          ))}
          {/* 就地批准、退回的结果：那一行已经不在队列里（或者不是选中的那一行）了，写在这里 */}
          {outcome ? (
            <p
              className="cockpit-note cockpit-queue__outcome"
              role={outcome.failed ? 'alert' : 'status'}
            >
              {outcome.text}
            </p>
          ) : null}
          {review && authSpan ? (
            <AuthCountdown
              className="cockpit-note cockpit-queue__auth"
              span={authSpan}
              now={review.auth.now}
              onExpire={review.auth.expire}
            />
          ) : null}
        </div>
        {timed ? <WaitLegend /> : null}
      </header>
      {split.active.length > 0 ? (
        <ol
          className="cockpit-queue__list"
          data-live={live ? 'true' : undefined}
        >
          {split.active.map((entry) => row(entry, false))}
        </ol>
      ) : entries.length === 0 && settled ? (
        <p className="cockpit-queue__clear" role="status">
          现在没有事等你。
        </p>
      ) : null}
      {split.later.length > 0 ? (
        <>
          <h3 className="cockpit-queue__later">
            稍后再看
            <span>{split.later.length} 件</span>
            <span>4 小时后，或它有了新进展，回到原位</span>
          </h3>
          <ol
            className="cockpit-queue__list"
            aria-label="稍后再看"
            data-live={live ? 'true' : undefined}
          >
            {split.later.map((entry) => row(entry, true))}
          </ol>
        </>
      ) : null}
      {entries.length > 0 ? (
        <p className="cockpit-queue__foot">
          <span>队列到底了</span>
          {asOf && timed ? <span>等了多久算到 {asOf}</span> : null}
        </p>
      ) : null}
      {entries.length > 0 && scope.live ? (
        <QueueKeys
          enabled={singleKeys}
          rejectVerb={rejectVerb}
          onShowAll={scope.showSheet}
        />
      ) : null}
    </section>
  );
}
