import { useId, useLayoutEffect, useRef } from 'react';
import { Button } from '../ui';
import { cx } from '../ui/dom';
import type { PreviewContent } from './previews';
import '../styles/policy.css';

/**
 * 不可逆动作的「执行预览」（设计规范第 8、9 节）：红色虚线框，出现在触发按钮上方。
 * 点触发按钮只打开这个框、不发任何请求；框里的「确认…」才走原来的流程，「取消」或 Esc 收起。
 * 出现时焦点移进框里（读屏读出「执行预览」与要做的事），收起时焦点还给打开它的按钮——
 * 只在焦点还留在框里时才还，人已经点到别处就不抢。
 */
export function ActionPreview({
  id,
  content,
  disabled,
  onConfirm,
  onCancel,
}: {
  id?: string;
  content: PreviewContent;
  /** 另一个动作在途时不许确认。 */
  disabled: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const labelId = useId();
  const titleId = useId();

  // 布局阶段移焦与归还：卸载时框还在文档里，才判断得出焦点是不是还在框内。
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const opener = document.activeElement;
    element.focus();
    return () => {
      const active = document.activeElement;
      if (
        opener instanceof HTMLElement &&
        opener !== element &&
        opener.isConnected &&
        (active === null ||
          active === document.body ||
          element.contains(active))
      )
        opener.focus();
    };
  }, []);

  return (
    <div
      ref={ref}
      id={id}
      className="policy-preview"
      role="group"
      aria-labelledby={`${labelId} ${titleId}`}
      tabIndex={-1}
      onKeyDown={(event) => {
        if (event.key !== 'Escape') return;
        event.preventDefault();
        event.stopPropagation();
        onCancel();
      }}
    >
      <p className="policy-preview__label" id={labelId}>
        执行预览
      </p>
      <p className="policy-preview__title" id={titleId}>
        {content.title}
      </p>
      <dl className="policy-preview__facts">
        {content.facts.map((fact) => (
          <div key={fact.label}>
            <dt>{fact.label}</dt>
            <dd className={cx(fact.signal && 'is-signal')}>
              {fact.signal ? (
                <span className="ui-signal-mark" aria-hidden="true" />
              ) : null}
              {fact.value}
            </dd>
          </div>
        ))}
      </dl>
      {content.notes.map((note) => (
        <p key={note} className="policy-preview__note">
          {note}
        </p>
      ))}
      <div className="policy-preview__actions">
        <Button
          variant="danger"
          size="sm"
          disabled={disabled}
          onClick={onConfirm}
        >
          {content.confirmLabel}
        </Button>
        <Button variant="text" size="sm" onClick={onCancel}>
          取消
        </Button>
      </div>
    </div>
  );
}
