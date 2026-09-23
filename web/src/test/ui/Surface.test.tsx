import { createRef } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { Surface } from '../../ui';

it('as="dialog" 时收对话框属性：onCancel 直接挂在 Surface 上，ref 是原生 dialog', () => {
  const ref = createRef<HTMLDialogElement>();
  const onCancel = vi.fn((event: { preventDefault: () => void }) =>
    event.preventDefault(),
  );
  render(
    <Surface
      as="dialog"
      ref={ref}
      open
      onCancel={onCancel}
      aria-label="附件信息"
      appear={false}
    >
      内容
    </Surface>,
  );
  const dialog = screen.getByRole('dialog', { name: '附件信息' });
  expect(dialog).toBe(ref.current);
  expect(dialog).toBeInstanceOf(HTMLDialogElement);
  expect(dialog).toHaveAttribute('open');
  expect(dialog).toHaveClass('ui-surface');
  const cancel = new Event('cancel', { cancelable: true });
  fireEvent(dialog, cancel);
  expect(onCancel).toHaveBeenCalledTimes(1);
  expect(cancel.defaultPrevented).toBe(true);
});

it('其余元素只收通用 HTML 属性，渲染不变：默认 div，出现动效类随 appear', () => {
  const ref = createRef<HTMLElement>();
  const { container, rerender } = render(
    <Surface ref={ref} className="file-op__review">
      审阅
    </Surface>,
  );
  const box = container.firstElementChild as HTMLElement;
  expect(box.tagName).toBe('DIV');
  expect(ref.current).toBe(box);
  expect(box).toHaveClass(
    'ui-surface',
    'ui-surface--appear',
    'file-op__review',
  );
  rerender(
    <Surface as="aside" appear={false} aria-label="侧栏">
      侧栏
    </Surface>,
  );
  const aside = screen.getByRole('complementary', { name: '侧栏' });
  expect(aside).toHaveClass('ui-surface');
  expect(aside).not.toHaveClass('ui-surface--appear');
});

// 类型层面：对话框属性只在 as="dialog" 时可用。由 tsc -b 把关——哪天放宽回通用属性，
// 下面的 @ts-expect-error 就会变成「未使用」而报错。运行时只确认渲染本身不受影响。
it('非 dialog 的 Surface 不收 onCancel', () => {
  const noop = () => {};
  // @ts-expect-error onCancel 只属于 as="dialog"
  const view = render(<Surface as="nav" onCancel={noop} />);
  expect(view.container.querySelector('nav')).toHaveClass('ui-surface');
});
