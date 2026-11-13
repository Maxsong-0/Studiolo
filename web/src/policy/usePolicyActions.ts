import {
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import {
  activateFailure,
  activatePolicy,
  needsReauthentication,
  revokeAgent,
  submitDesired,
  validateDesired,
  withdrawDesired,
} from '../policyApi';
import type {
  ActivateOutcome,
  ActivateRequest,
  DesiredInput,
  DesiredRecord,
  RevokeRequest,
} from '../policyModel';
import { LEVEL_LABEL, STATUS_LABEL, shortHash } from '../policyModel';
import { PolicyChangedContext } from './policyChanged';
import { publishedNotice, revokePreview } from './previews';
import type { PreviewContent } from './previews';

/** 反馈落在发起动作的那一块：`queue`、`editor`、`access` 这类名字由面板自己定。 */
export type Feedback = { origin: string; text: string };
/**
 * 一次要二次认证的动作（发布、回滚、解除全局撤销）：请求本体、成功提示里的名字、给人看的动作说明、
 * 发起的那一块，以及执行预览里列出的同一对象其余未发布草案份数：会被一并标为已取代的较早草案、
 * 照旧待处理的更新草案（只有发布草案才可能不是 0）。
 */
export type Activation = {
  request: ActivateRequest;
  label: string;
  action: string;
  origin: string;
  supersedes: number;
  remains: number;
};
/** 被二次认证挡下、等人就地认证后原样重试的那一个动作；`attempt` 只用于措辞。 */
export type PendingStepUp = Activation & { attempt: number };
/**
 * 打开着的执行预览：同一时刻只有一个，`key` 标明属于哪个按钮
 * （`publish:<草案号>`、`rollback:<版本号>`、`release`、`revoke`）；请求在打开时就定下，确认时原样发出。
 */
export type OpenPreview = { key: string; content: PreviewContent } & (
  | { kind: 'activate'; activation: Activation }
  | { kind: 'revoke'; origin: string; request: RevokeRequest }
);
/** 收场时服务端状态没变（被二次认证挡下，或只是读了一版正文），不必重读，也不算写过。 */
export type Unchanged = { notice: string; reload: false };

/** 撤销接入那一个执行预览的 key：整页只有一张撤销接入表单。 */
export const REVOKE_PREVIEW = 'revoke';

export interface PolicyActionOptions {
  /**
   * 发布、回滚或解除全局撤销刚成功、还没重读时调用：策略页借它收起选中的草案和在读的版本——
   * 版本列表已经变了，位置里那个号不再是「刚才在看的那一份」。
   */
  onActivated?: (outcome: ActivateOutcome) => void;
}

/**
 * 策略平面的写动作，全站只有这一份（T-198）：策略页、规则页、驾驶舱的行内批准都用它。
 * 同一时刻只做一件；每件做完重读整份数据（`reload`）。发布、回滚、解除全局撤销和撤销接入先展开执行预览，
 * 确认才发请求；被 403 `reauthentication_required` 挡下时记下这一次，等人在 PolicyStepUp 里认证后原样重试——
 * 人审过哪一份生效中内容就按哪一份提交，期间变了由服务端以 hash_mismatch 挡下，绝不悄悄改用新的。
 * 控制器挂在视图上：面板切换、编辑器换规则都不会中断在途的动作。
 * 每做完一件还会发一次「策略平面刚被写过」（PolicyChangedContext），顶栏角标用的那份草案列表跟着重读。
 */
export function usePolicyActions(
  reload: () => Promise<void>,
  options: PolicyActionOptions = {},
) {
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<Feedback | null>(null);
  const [error, setError] = useState<Feedback | null>(null);
  const [pending, setPending] = useState<PendingStepUp | null>(null);
  const [preview, setPreview] = useState<OpenPreview | null>(null);
  // 控制器在挂载时重建：StrictMode 的装载—卸载—再装载会先 abort 一次，沿用同一个已中止的控制器
  // 会让此后每个动作在发请求前就被取消，界面表现为点了毫无反应（BUG-016）。
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    lifetime.current = new AbortController();
    return () => lifetime.current.abort();
  }, []);
  const changed = useContext(PolicyChangedContext);
  // 回调每次重画都可能是新的：动作结束时调最新的那一份。
  const latest = useRef({ options, changed });
  useLayoutEffect(() => {
    latest.current = { options, changed };
  });

  /**
   * 做一件事。`operation` 的返回值是要显示的一句话；返回 `{ reload: false }` 用于服务端明确没有改动状态的
   * 收场（被二次认证挡下、只读了一版正文）：被挡下时再重读只会让 busy 多挂一会儿，把人认证完的重试吞掉。
   * `after` 在成功并重读之后调用（如新规则保存后改位置）。任何动作开始都收起打开着的执行预览：
   * 预览只描述发起那一刻的状态。
   */
  async function run(
    origin: string,
    operation: (signal: AbortSignal) => Promise<string | Unchanged>,
    after?: () => void,
  ) {
    if (busy) return;
    const signal = lifetime.current.signal;
    setBusy(true);
    setError(null);
    setNotice(null);
    setPreview(null);
    try {
      const result = await operation(signal);
      // 这一件真的做了（不是被二次认证挡下）：先告诉全站那一份草案读取——哪怕人已经离开这一页，写入也是真的。
      if (typeof result === 'string') latest.current.changed();
      if (signal.aborted) return;
      setNotice({
        origin,
        text: typeof result === 'string' ? result : result.notice,
      });
      if (typeof result === 'string') await reload();
      if (!signal.aborted) after?.();
    } catch (cause) {
      if (signal.aborted) return;
      setError({
        origin,
        text:
          cause instanceof Error ? cause.message : '操作未完成，请重新核对。',
      });
      // 没做成也可能已经写了一半（先交新草案、撤旧的那一步失败）：同样重读。
      latest.current.changed();
      await reload().catch(() => undefined);
    } finally {
      if (!signal.aborted) setBusy(false);
    }
  }

  function submit(origin: string, input: DesiredInput, after?: () => void) {
    void run(
      origin,
      async (signal) => {
        const record = await submitDesired(input, signal);
        return record.report?.ok
          ? '已提交规则草案并校验通过；发布前请逐条确认扩大项。'
          : '已提交规则草案，但校验未通过；请按逐条问题修改后重新提交。';
      },
      after,
    );
  }

  /**
   * 在待发布草案上追加改动：先交新草案，成功后再撤回旧的。先交后撤——撤回失败最多多留一份旧草案
   * （发布新草案时后端会把它标为已取代），而不会出现旧的撤了、新的没交上、前面的改动丢了。
   */
  function amend(origin: string, input: DesiredInput, previous: DesiredRecord) {
    void run(origin, async (signal) => {
      const record = await submitDesired(input, signal);
      await withdrawDesired(previous.id, signal);
      return record.report?.ok
        ? '已在待发布草案上追加并校验通过，旧草案已撤回；发布前请逐条确认扩大项。'
        : '已在待发布草案上追加，但校验未通过；旧草案已撤回，请按逐条问题修改后重新提交。';
    });
  }

  function validate(origin: string, record: DesiredRecord) {
    void run(origin, async (signal) => {
      const updated = await validateDesired(record.id, signal);
      return `已重新校验：${STATUS_LABEL[updated.status]}。`;
    });
  }

  function withdraw(origin: string, record: DesiredRecord, after?: () => void) {
    void run(
      origin,
      async (signal) => {
        await withdrawDesired(record.id, signal);
        return '已撤回这份草案。';
      },
      after,
    );
  }

  /** 三级撤销接入：即刻生效，本身不走二次认证。 */
  function revoke(origin: string, request: RevokeRequest) {
    void run(origin, async (signal) => {
      const outcome = await revokeAgent(request, signal);
      return outcome.created
        ? `已撤销接入：${LEVEL_LABEL[outcome.level]} ${outcome.target}（即刻生效）。`
        : `${LEVEL_LABEL[outcome.level]} ${outcome.target} 已处于撤销接入中。`;
    });
  }

  // 认证后重试沿用同一个 request：人审过哪一份生效中内容就按哪一份提交。
  function activate(activation: Activation, attempt = 0) {
    const { request, label, action, origin, supersedes, remains } = activation;
    void run(origin, async (signal) => {
      try {
        const outcome = await activatePolicy(request, signal);
        latest.current.options.onActivated?.(outcome);
        const audit = outcome.audit_confirmed
          ? ''
          : '（审计未确认，请核对账本）';
        if (outcome.kind === 'released') return `已解除全局撤销${audit}。`;
        return `${label}：已发布为 v${outcome.activation.version}，生效中摘要 ${shortHash(
          outcome.activation.effective_hash,
        )}${
          outcome.activation.cleared_type_revocation
            ? '，并解除了该类型的撤销接入'
            : ''
        }${publishedNotice(supersedes, remains)}${audit}。`;
      } catch (cause) {
        // 403「需要二次认证」不是终点：动作没有发生，只差一次一次性验证码。
        if (needsReauthentication(cause)) {
          setPending({ ...activation, attempt });
          return {
            notice: `${action} 需要二次认证：请在弹出的窗口输入一次性验证码，通过后会自动继续这一次动作。`,
            reload: false,
          };
        }
        throw new Error(activateFailure(cause));
      }
    });
  }

  /** 打开一个要二次认证的动作的执行预览；触发按钮再点一次就收起。 */
  function togglePreview(
    key: string,
    activation: Activation,
    content: PreviewContent,
  ) {
    setPreview((current) =>
      current?.key === key
        ? null
        : { key, content, kind: 'activate', activation },
    );
  }

  /** 打开撤销接入的执行预览：请求由表单校验后交来。 */
  function previewRevoke(origin: string, request: RevokeRequest) {
    setPreview({
      key: REVOKE_PREVIEW,
      content: revokePreview(request),
      kind: 'revoke',
      origin,
      request,
    });
  }

  function confirmPreview() {
    const current = preview;
    if (!current) return;
    setPreview(null);
    if (current.kind === 'activate') activate(current.activation);
    else revoke(current.origin, current.request);
  }

  /** 只收起符合条件的那一个执行预览（表单改了字段、换了选中的草案）。 */
  function closePreviewIf(matches: (open: OpenPreview) => boolean) {
    setPreview((current) => (current && matches(current) ? null : current));
  }

  function retryAfterStepUp() {
    const next = pending;
    setPending(null);
    if (next) activate(next, next.attempt + 1);
  }

  return {
    busy,
    notice,
    error,
    pending,
    preview,
    run,
    submit,
    amend,
    validate,
    withdraw,
    togglePreview,
    previewRevoke,
    confirmPreview,
    // 不收参数：常被直接当点击回调用，事件对象不能被当成别的东西。
    closePreview: () => setPreview(null),
    closePreviewIf,
    retryAfterStepUp,
    closeStepUp: () => setPending(null),
  };
}

export type PolicyActions = ReturnType<typeof usePolicyActions>;
