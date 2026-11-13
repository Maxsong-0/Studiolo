import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { PolicyNow } from '../policy/PolicyNow';
import { PolicyQueue } from '../policy/PolicyQueue';
import { PolicySubmit } from '../policy/PolicySubmit';
import {
  policyPanelSlot,
  policyTab,
  readingVersion,
  selectedDraft,
  wantsQueueFocus,
  type PolicyTab,
} from '../policy/policyLocation';
import { EMPTY_SUBMIT_DRAFT, type SubmitDraft } from '../policy/submitDraft';
import { usePolicyActions } from '../policy/usePolicyActions';
import { useVersionBodies } from '../policy/useVersionBodies';
import { readPolicySnapshot, readVersion, submitDesired } from '../policyApi';
import type {
  DesiredInput,
  ListedDesired,
  PolicyStatus,
  VersionMeta,
} from '../policyModel';
import { isOpenStatus } from '../policyModel';
import { Tabs } from '../ui';
import { useViewLocation } from '../useViewLocation';
import { PolicyStepUp } from './PolicyStepUp';
import '../styles/policy.css';
import '../styles/policy-tabs.css';

type Loaded = {
  status: PolicyStatus;
  desired: ListedDesired[];
  versions: VersionMeta[];
};

/** 三个子标签，顺序就是标签栏的顺序。 */
const TABS: readonly { id: PolicyTab; label: string }[] = [
  { id: 'now', label: '现状' },
  { id: 'pending', label: '待处理' },
  { id: 'submit', label: '提交' },
];

/**
 * 策略平面视图（T-198；设计规范第 3、9 节）：一排子标签，每个标签只做一件事——
 * 「现状」现在生效的是什么、版本与回滚、撤销接入；「待处理」草案队列和选中那一份的细节与发布；
 * 「提交」写一份新草案。全部动作先校验后动手，永不自动发布；发布、回滚、解除全局撤销与三级撤销接入
 * 先在按钮上方展开「执行预览」，确认后才发请求。
 *
 * 这里只管四样：整份数据和它的重读、在看哪个标签和选中哪一项（都写在位置里）、「提交」里正在写的草稿、
 * 以及全页那一份动作调度（usePolicyActions）连同二次认证弹层。内容在 policy/ 下的三个面板里。
 */
export function PolicyPanel({ initial }: { initial: Loaded }) {
  const [data, setData] = useState<Loaded>(initial);
  // 「提交」里正在写的那一份：放在这里，换标签不丢。
  const [draft, setDraft] = useState<SubmitDraft>(EMPTY_SUBMIT_DRAFT);
  const ids = useId();
  const queueRef = useRef<HTMLElement>(null);
  // 在看哪个标签、选中哪份草案、在读哪一版都属于「位置」：重挂、刷新、分享链接后回到同一处。
  // 位置里只有标签名、草案号、版本号；版本正文与草案正文绝不进 URL。
  const { location, goTo } = useViewLocation();
  const tab = policyTab(location);
  // 换标签时各标签记住自己选中的那一项，回来时还在；只记在内存里，位置里始终只有当前标签的那一项。
  const kept = useRef<Partial<Record<PolicyTab, string | null>>>({});
  // 控制器必须在挂载时重建：StrictMode 的装载—卸载—再装载会先 abort 一次，沿用同一个
  // 已中止的控制器会让此后每次重读在发请求前就被取消（BUG-016）。
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    lifetime.current = new AbortController();
    return () => lifetime.current.abort();
  }, []);

  const reload = useCallback(async () => {
    const signal = lifetime.current.signal;
    const snapshot = await readPolicySnapshot(signal);
    if (!signal.aborted) setData(snapshot);
  }, []);

  const actions = usePolicyActions(reload, {
    // 发布、回滚、解除全局撤销成功：队列和版本列表都变了，位置里那一项不再是「刚才在看的那一份」。
    onActivated: () => {
      kept.current = {};
      goTo({ view: 'policy', panel: policyPanelSlot(tab), item: null });
    },
  });
  const versionBodies = useVersionBodies();
  const { bodies, read: readBody, store: storeBody } = versionBodies;

  const { status, desired, versions } = data;
  const actionable = desired.filter((record) => isOpenStatus(record.status));
  const selected = selectedDraft(location, actionable);

  // 位置里带着版本号就把那一份正文读回来（会话重新核对导致整页重挂、刷新、或别人打开同一个链接）。
  // 读失败就停在列表：不弹通知、不动 URL、不猜别的版本。
  const wanted = readingVersion(location);
  const wantedVersion =
    wanted === null
      ? null
      : (versions.find((row) => String(row.version) === wanted)?.version ??
        null);
  useEffect(() => {
    if (wantedVersion !== null) void readBody(wantedVersion);
  }, [wantedVersion, readBody]);
  const readingBody =
    wantedVersion === null ? null : (bodies.get(wantedVersion) ?? null);
  const reading = readingBody?.kind === 'ready' ? readingBody.record : null;

  // 旧写法 `item=desired`（⌘K 的「打开策略待处理队列」「发布策略」，和拆标签之前的链接）：
  // 把待处理草案队列滚进视野并聚焦（不改写位置、不报错）。只在位置变成它的那一刻做一次，
  // 刷新列表不会把焦点再拽回来；别的对话框开着就只滚不抢焦点。
  const toQueue = wantsQueueFocus(location);
  useEffect(() => {
    if (!toQueue) return;
    const queue = queueRef.current;
    if (!queue) return;
    queue.scrollIntoView({ block: 'start' });
    if (!document.querySelector('dialog[open]'))
      queue.focus({ preventScroll: true });
  }, [toQueue]);

  function show(next: PolicyTab) {
    if (next === tab) return;
    // 旧写法的 `desired` 只是带路的记号，不是选中的草案，不记。
    kept.current[tab] = toQueue ? null : location.item;
    // 执行预览只描述发起那一刻眼前的东西：人走开了就收起。
    actions.closePreview();
    goTo({
      view: 'policy',
      panel: policyPanelSlot(next),
      item: kept.current[next] ?? null,
    });
  }

  function pick(record: ListedDesired) {
    // 换了草案，发布预览描述的已不是眼前这一份。
    actions.closePreviewIf((open) => open.key.startsWith('publish:'));
    goTo({
      view: 'policy',
      panel: policyPanelSlot('pending'),
      item: record.id,
    });
  }

  function openVersion(meta: VersionMeta) {
    void actions.run('versions', async (signal) => {
      // 先记下再写位置：上面的回读据此认出这一份已经在手，不重复请求。
      storeBody(await readVersion(meta.version, signal));
      goTo({
        view: 'policy',
        panel: policyPanelSlot('now'),
        item: String(meta.version),
      });
      // 只是读了一版正文：服务端什么都没变，不必重读整份数据，也不算「策略平面刚被写过」。
      return { notice: `已读取 v${meta.version}。`, reload: false };
    });
  }

  function submit(input: DesiredInput) {
    let created: string | null = null;
    void actions.run(
      'submit',
      async (signal) => {
        const record = await submitDesired(input, signal);
        created = record.id;
        return record.report?.ok
          ? `已提交并校验通过（${record.id}）；发布前请逐条确认扩大项。`
          : `已提交但校验未通过（${record.id}）；请按逐条问题修改后重新提交。`;
      },
      () => {
        // 列表已经重读过：把人带到「待处理」里刚交的那一份，结果那一句跟着过去。
        if (created === null) return;
        goTo({
          view: 'policy',
          panel: policyPanelSlot('pending'),
          item: created,
        });
      },
    );
  }

  return (
    <div className="policy">
      <div className="policy-bar">
        <Tabs
          items={TABS.map(({ id, label }) =>
            id === 'pending'
              ? { id, label, count: actionable.length }
              : { id, label },
          )}
          value={tab}
          onChange={(next) => {
            const target = TABS.find(({ id }) => id === next);
            if (target) show(target.id);
          }}
          ariaLabel="策略面板"
          idPrefix={ids}
          className="policy-bar__tabs"
        />
        <p className="policy-bar__summary">
          {/* 全局撤销是要处理的事：三个标签上都看得到，解除在「现状」里。 */}
          {status.global_revoked ? (
            <span className="policy-bar__alert">
              <span className="ui-signal-mark" aria-hidden="true" />
              全局撤销中
            </span>
          ) : null}
          <span>生效 {status.entries.length} 项</span>
          <span>{versions.length} 版</span>
        </p>
      </div>
      <div
        className="policy-panel"
        role="tabpanel"
        id={`${ids}-panel-${tab}`}
        aria-labelledby={`${ids}-tab-${tab}`}
      >
        {tab === 'now' ? (
          <PolicyNow
            status={status}
            versions={versions}
            actions={actions}
            reading={reading}
            onOpenVersion={openVersion}
          />
        ) : tab === 'pending' ? (
          <PolicyQueue
            status={status}
            desired={desired}
            actionable={actionable}
            selected={selected}
            actions={actions}
            versions={versionBodies}
            queueRef={queueRef}
            onPick={pick}
          />
        ) : (
          <PolicySubmit
            status={status}
            draft={draft}
            setDraft={setDraft}
            actions={actions}
            versions={versionBodies}
            onSubmit={submit}
          />
        )}
      </div>

      {actions.pending ? (
        <PolicyStepUp
          action={actions.pending.action}
          attempt={actions.pending.attempt}
          onAuthenticated={actions.retryAfterStepUp}
          onClose={actions.closeStepUp}
        />
      ) : null}
    </div>
  );
}
