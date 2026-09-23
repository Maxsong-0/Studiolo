import type { ComponentPropsWithRef } from 'react';
import { cx } from './dom';
import './ui.css';

export type ButtonVariant = 'primary' | 'secondary' | 'danger' | 'text';
export type ButtonSize = 'md' | 'sm';

export type ButtonProps = ComponentPropsWithRef<'button'> & {
  /** primary 黑底反白；secondary 1.5px 黑框（缺省）；danger 1.5px 红框红字；text 纯文字。 */
  variant?: ButtonVariant;
  size?: ButtonSize;
};

/** 按钮。type 缺省为 button，免得在表单里被当成提交。 */
export function Button({
  variant = 'secondary',
  size = 'md',
  type = 'button',
  className,
  ...rest
}: ButtonProps) {
  return (
    <button
      {...rest}
      type={type}
      className={cx(
        'ui-button',
        `ui-button--${variant}`,
        `ui-button--${size}`,
        className,
      )}
    />
  );
}
