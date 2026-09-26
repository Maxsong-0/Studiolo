import { useEffect, useRef, useState } from 'react';
import {
  activateFailure,
  activatePolicy,
  needsReauthentication,
  submitDesired,
  validateDesired,
  withdrawDesired,
} from '../policyApi';
import type {
  ActivateRequest,
  DesiredInput,
  DesiredRecord,
} from '../policyModel';
import { STATUS_LABEL, shortHash } from '../policyModel';
import type { PreviewContent } from '../policy/previews';

/** 反馈落在发起动作的那一块：`editor`、`access` 这类名字由面板自己定。 */
export type Feedback = { origin: string; text: string };
/** 一次 L3 发布：请求本体、成功提示里的名字、给人看的动作说明、发起的那一块。 */
export type Activation = {
  request: ActivateRequest;
  label: string;
  action: string;
  origin: string;
};
/** 被二次认证挡下、等人就地认证后原样重试的那一次发布；`attempt` 只用于措辞。 */
export type PendingStepUp = Activation & { attempt: number };
/** 打开着的执行预览：同一时刻只有一个，`key` 标明属于哪个按钮；请求在打开时就定下，确认时原样发出。 */
export type OpenPreview = {
  key: string;
  content: PreviewContent;
  activation: Activation;
};
/** 收场时服务端状态没变（被二次认证挡下），不必重读。 */
type Unchanged = { notice: string; reload: false };

/**
 * 规则视图的写动作（与策略页 PolicyPanel 同一套流程）：提交规则草案、重新校验、撤回、发布。
 * 同一时刻只做一件；每件做完重读整份数据（`reload`）。发布先展开执行预览，确认才发请求；
 * 被 403 `reauthentication_required` 挡下时记下这一次，等人在 PolicyStepUp 里认证后原样重试——
 * 人审过哪一份生效中内容就按哪一份发布，期间变了由服务端以 hash_mismatch 挡下，绝不悄悄改用新的。
 * 控制器挂在视图上：面板切换、编辑器换规则都不会中断在途的动作。
 */
export function usePolicyActions(reload: () => Promise<void>) {
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<Feedback | null>(null);
  const [error, setError] = useState<Feedback | null>(null);
  const [pending, setPending] = useState<PendingStepUp | null>(null);
  const [preview, setPreview] = useState<OpenPreview | null>(null);
  // 控制器在挂载时重建：StrictMode 的装载—卸载—再装载会先 abort 一次（BUG-016 的教训）。
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    lifetime.current = new AbortController();
    return () => lifetime.current.abort();
  }, []);

  /** 返回值是要显示的一句话；`after` 在成功并重读之后调用（如新规则保存后改位置）。 */
  async function act(
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
      await reload().catch(() => undefined);
    } finally {
      if (!signal.aborted) setBusy(false);
    }
  }

  function submit(origin: string, input: DesiredInput, after?: () => void) {
    void act(
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

  function validate(origin: string, record: DesiredRecord) {
    void act(origin, async (signal) => {
      const updated = await validateDesired(record.id, signal);
      return `已重新校验：${STATUS_LABEL[updated.status]}。`;
    });
  }

  function withdraw(origin: string, record: DesiredRecord) {
    void act(origin, async (signal) => {
      await withdrawDesired(record.id, signal);
      return '已撤回这份草案。';
    });
  }

  function activate(activation: Activation, attempt = 0) {
    const { request, label, action, origin } = activation;
    void act(origin, async (signal) => {
      try {
        const outcome = await activatePolicy(request, signal);
        const audit = outcome.audit_confirmed
          ? ''
          : '（审计未确认，请核对账本）';
        if (outcome.kind === 'released') return `已解除全局撤销${audit}。`;
        return `${label}：已发布为 v${outcome.activation.version}，生效中摘要 ${shortHash(
          outcome.activation.effective_hash,
        )}${audit}。`;
      } catch (cause) {
        // 403「需要二次认证」不是终点：发布没有发生，只差一次一次性验证码。
        if (needsReauthentication(cause)) {
          setPending({ request, label, action, origin, attempt });
          return {
            notice: `${action} 需要二次认证：请在弹出的窗口输入一次性验证码，通过后会自动继续这一次发布。`,
            reload: false,
          };
        }
        throw new Error(activateFailure(cause));
      }
    });
  }

  /** 触发按钮再点一次就收起执行预览。 */
  function togglePreview(
    key: string,
    activation: Activation,
    content: PreviewContent,
  ) {
    setPreview((current) =>
      current?.key === key ? null : { key, content, activation },
    );
  }

  function confirmPreview() {
    const current = preview;
    if (!current) return;
    setPreview(null);
    activate(current.activation);
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
    submit,
    validate,
    withdraw,
    togglePreview,
    confirmPreview,
    closePreview: () => setPreview(null),
    retryAfterStepUp,
    closeStepUp: () => setPending(null),
  };
}

export type PolicyActions = ReturnType<typeof usePolicyActions>;
