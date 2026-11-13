import { useId, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import type { RevokeRequest, RevocationLevel } from '../policyModel';
import { LEVEL_LABEL } from '../policyModel';
import { POLICY_TERMS } from '../policy/terms';
import { Button } from '../ui';
import '../styles/policy.css';

/**
 * 字段名：等宽小字标签；给了原术语就跟在人话后面（「对象 subject」）。
 * 原术语放在 label 外面：输入框的可访问名只是人话。提交草案的表单（policy/PolicySubmit）也用它。
 */
export function FieldLabel({
  htmlFor,
  raw,
  children,
}: {
  htmlFor: string;
  raw?: string;
  children: ReactNode;
}) {
  return (
    <span className="policy-field__label">
      <label htmlFor={htmlFor}>{children}</label>
      {/* 真空格：线性朗读读作「对象 subject」；在 flex 里不占位，间距仍由 gap 给 */}
      {raw ? (
        <>
          {' '}
          <code className="policy-term" lang="en">
            {raw}
          </code>
        </>
      ) : null}
    </span>
  );
}

/**
 * 三级撤销接入。提交只打开执行预览（渲染在提交按钮上方），预览里确认才发出；
 * 预览打开后改了任何字段，预览随之作废，免得确认的不是眼前这一份。
 */
export function RevokeForm({
  busy,
  preview,
  previewId,
  onRequest,
  onEdit,
}: {
  busy: boolean;
  /** 打开的执行预览；没有就是 null。 */
  preview: ReactNode;
  /** 预览框的 id：提交按钮展开时用 aria-controls 指向它。 */
  previewId: string;
  /** 本地校验通过的请求：交给上层打开执行预览。 */
  onRequest: (request: RevokeRequest) => void;
  /** 任一字段改动。 */
  onEdit: () => void;
}) {
  const id = useId();
  const [level, setLevel] = useState<RevocationLevel>('instance');
  const [error, setError] = useState('');
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const values = new FormData(event.currentTarget);
    const target = String(values.get('target') ?? '').trim();
    const reason = String(values.get('reason') ?? '').trim();
    if (level !== 'global' && !target) {
      setError('请填写要撤销接入的实例 id 或类型名。');
      return;
    }
    if (!reason) {
      setError('请填写撤销理由（进权威审计）。');
      return;
    }
    setError('');
    onRequest(
      level === 'global' ? { level, reason } : { level, target, reason },
    );
  }
  const previewing = preview !== null && preview !== undefined;
  return (
    <form
      className="policy-form"
      onSubmit={submit}
      onChange={onEdit}
      aria-label={POLICY_TERMS.revoke.human}
    >
      <div className="policy-form__row">
        <div className="policy-field">
          <FieldLabel htmlFor={`${id}-level`}>级别</FieldLabel>
          <span className="policy-select">
            <select
              id={`${id}-level`}
              name="level"
              value={level}
              onChange={(event) =>
                setLevel(event.target.value as RevocationLevel)
              }
            >
              {(Object.keys(LEVEL_LABEL) as RevocationLevel[]).map((value) => (
                <option key={value} value={value}>
                  {LEVEL_LABEL[value]}
                </option>
              ))}
            </select>
          </span>
        </div>
        <div className="policy-field">
          <FieldLabel htmlFor={`${id}-target`}>目标</FieldLabel>
          <input
            id={`${id}-target`}
            className="policy-input policy-input--mono"
            name="target"
            type="text"
            autoComplete="off"
            spellCheck={false}
            disabled={level === 'global'}
            placeholder={
              level === 'global'
                ? '全局不需要目标'
                : level === 'type'
                  ? 'claude'
                  : '实例 id（ULID）'
            }
          />
        </div>
      </div>
      <div className="policy-field">
        <FieldLabel htmlFor={`${id}-reason`}>理由</FieldLabel>
        <input
          id={`${id}-reason`}
          className="policy-input"
          name="reason"
          type="text"
          autoComplete="off"
        />
      </div>
      {error ? (
        <p className="policy-form__error" role="alert">
          {error}
        </p>
      ) : null}
      {preview}
      <div className="policy-actions">
        <Button
          type="submit"
          variant="danger"
          disabled={busy}
          aria-expanded={previewing}
          aria-controls={previewing ? previewId : undefined}
        >
          撤销接入（即刻生效）
        </Button>
      </div>
      <p className="policy-hint">
        恢复只走发布：重新发布该类型的能力集即解除类型撤销；解除全局撤销需二次认证。
      </p>
    </form>
  );
}
