import { StrictMode, useRef } from 'react';
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import * as motion from 'motion';
import { useDialogSurface } from '../useDialogSurface';
import {
  atRenderedFrame,
  cancel,
  entered,
  mediaPreference,
  surface,
} from './dialogSurfaceHelpers';

vi.mock('motion', async (importOriginal) => {
  const actual = await importOriginal<typeof import('motion')>();
  const animate = Object.assign(vi.fn(actual.animate), actual.animate);
  return { ...actual, animate };
});

function Surface({ onClose = vi.fn() }: { onClose?: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const { dismiss } = useDialogSurface(ref, onClose);
  return (
    <dialog
      ref={ref}
      aria-label="测试弹窗"
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
    >
      <button data-dialog-dismiss onClick={dismiss}>
        关闭
      </button>
      <button>继续操作</button>
      <details>
        <summary>展开大纲</summary>
      </details>
      <button disabled>不可操作</button>
      <fieldset disabled>
        <button>禁用分组</button>
      </fieldset>
      <button aria-disabled="true">语义禁用</button>
      <input aria-label="输入" />
    </dialog>
  );
}

it('无偏好查询时立即打开，取消后仅关闭一次并归还仍连接的焦点', () => {
  vi.stubGlobal('matchMedia', undefined);
  const launcher = document.createElement('button');
  document.body.append(launcher);
  launcher.focus();
  const close = vi.fn();
  const view = render(<Surface onClose={close} />);
  const dialog = surface();
  expect(dialog).toHaveAttribute('open');
  expect(dialog.style.transform).toBe('none');
  screen.getByRole('button', { name: '关闭' }).focus();
  cancel(dialog);
  cancel(dialog);
  expect(close).toHaveBeenCalledTimes(1);
  expect(dialog).not.toHaveAttribute('open');
  expect(launcher).toHaveFocus();
  view.unmount();
  launcher.remove();
});

it('数值动画不依赖 Web Animations，入场未完成时关闭不跳回完整显示', async () => {
  mediaPreference(false);
  const close = vi.fn();
  render(<Surface onClose={close} />);
  const dialog = surface();
  expect(dialog).toHaveAttribute('open');
  expect(dialog.style.opacity).toBe('0');
  expect(dialog.style.transform).toBe('translateY(12px) scale(0.97)');
  await waitFor(
    () => {
      const opacity = Number(dialog.style.opacity);
      expect(opacity).toBeGreaterThan(0);
      expect(opacity).toBeLessThan(1);
    },
    { interval: 5 },
  );
  atRenderedFrame(() => {
    const visible = dialog.style.opacity;
    fireEvent.click(screen.getByRole('button', { name: '关闭' }));
    expect(dialog.style.opacity).toBe(visible);
  });
  expect(close).not.toHaveBeenCalled();
  cancel(dialog);
  await waitFor(() => expect(close).toHaveBeenCalledTimes(1), {
    timeout: 1500,
  });
  expect(dialog).not.toHaveAttribute('open');
});

it('退场中重复关闭不重启动画，禁用控件与 Tab 也不取消退出', async () => {
  mediaPreference(false);
  const animations = vi.mocked(motion.animate);
  const close = vi.fn();
  render(<Surface onClose={close} />);
  await entered();
  const dialog = surface();
  const button = screen.getByRole('button', { name: '关闭' });
  fireEvent.click(button);
  fireEvent.pointerDown(button);
  fireEvent.click(button);
  for (const name of ['不可操作', '禁用分组', '语义禁用']) {
    fireEvent.pointerDown(screen.getByRole('button', { name }));
  }
  fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Tab' });
  cancel(dialog);
  expect(animations).toHaveBeenCalledTimes(2);
  await waitFor(() => expect(close).toHaveBeenCalledTimes(1), {
    timeout: 1500,
  });
});

it.each(['按钮', '折叠标题点击', '折叠标题回车'])(
  '%s 可反向打断退场，旧回调无效且 Esc 仍可关闭',
  async (interaction) => {
    mediaPreference(false);
    const animations = vi.mocked(motion.animate);
    const close = vi.fn();
    render(<Surface onClose={close} />);
    await entered();
    const dialog = surface();
    fireEvent.click(screen.getByRole('button', { name: '关闭' }));
    const oldCompletion = animations.mock.calls[1][2]?.onComplete;
    expect(oldCompletion).toBeTypeOf('function');
    await waitFor(() => expect(Number(dialog.style.opacity)).toBeLessThan(1), {
      interval: 5,
    });
    atRenderedFrame(() => {
      const visible = dialog.style.opacity;
      const control =
        interaction === '按钮'
          ? screen.getByRole('button', { name: '继续操作' })
          : screen.getByText('展开大纲');
      if (interaction === '折叠标题回车')
        fireEvent.keyDown(control, { key: 'Enter' });
      else fireEvent.click(control);
      expect(dialog.style.opacity).toBe(visible);
    });
    act(() => {
      oldCompletion?.();
    });
    expect(dialog).toHaveAttribute('open');
    expect(close).not.toHaveBeenCalled();
    await entered();
    expect(close).not.toHaveBeenCalled();
    cancel(dialog);
    await waitFor(() => expect(close).toHaveBeenCalledTimes(1), {
      timeout: 1500,
    });
  },
);

it('输入可取消退出，动态减少效果立即移除位移并完成下一次关闭', async () => {
  const preference = mediaPreference(false);
  const close = vi.fn();
  render(<Surface onClose={close} />);
  await entered();
  const dialog = surface();
  fireEvent.click(screen.getByRole('button', { name: '关闭' }));
  fireEvent.keyDown(screen.getByRole('textbox'), { key: '文' });
  preference.change(true);
  expect(dialog.style.transform).toBe('none');
  expect(dialog.style.opacity).toBe('1');
  expect(close).not.toHaveBeenCalled();
  cancel(dialog);
  expect(close).toHaveBeenCalledTimes(1);
});

it('退场中启用减少效果立即完成关闭且回调采用最新值', async () => {
  const preference = mediaPreference(false);
  const oldClose = vi.fn();
  const close = vi.fn();
  const view = render(<Surface onClose={oldClose} />);
  await entered();
  const dialog = surface();
  view.rerender(<Surface onClose={close} />);
  cancel(dialog);
  preference.change(true);
  expect(close).toHaveBeenCalledTimes(1);
  expect(oldClose).not.toHaveBeenCalled();
  expect(dialog).not.toHaveAttribute('open');
});

it('卸载停止退场与偏好监听，不再触发关闭也不聚焦已移除的启动元素', async () => {
  const preference = mediaPreference(false);
  const launcher = document.createElement('button');
  document.body.append(launcher);
  launcher.focus();
  const focus = vi.spyOn(launcher, 'focus');
  const close = vi.fn();
  const view = render(<Surface onClose={close} />);
  const dialog = surface();
  cancel(dialog);
  launcher.remove();
  view.unmount();
  expect(dialog).not.toHaveAttribute('open');
  expect(preference.remove).toHaveBeenCalledWith(
    'change',
    expect.any(Function),
  );
  preference.change(true);
  await act(async () => {
    await Promise.resolve();
  });
  expect(close).not.toHaveBeenCalled();
  expect(focus).not.toHaveBeenCalled();
});

it('StrictMode 重建动画资源后仍可正常打开与关闭', () => {
  mediaPreference(true);
  const close = vi.fn();
  render(
    <StrictMode>
      <Surface onClose={close} />
    </StrictMode>,
  );
  const dialog = surface();
  expect(dialog).toHaveAttribute('open');
  cancel(dialog);
  expect(close).toHaveBeenCalledTimes(1);
});

it('关闭减少效果后不重放入场，后续关闭恢复可中断动画', async () => {
  const preference = mediaPreference(true);
  const close = vi.fn();
  render(<Surface onClose={close} />);
  const dialog = surface();
  preference.change(false);
  expect(dialog.style.opacity).toBe('1');
  expect(dialog.style.transform).toBe('translateY(0px) scale(1)');
  cancel(dialog);
  expect(close).not.toHaveBeenCalled();
  await waitFor(() => expect(close).toHaveBeenCalledTimes(1), {
    timeout: 1500,
  });
});

it('卸载不从已经接管焦点的另一弹窗抢回启动按钮', () => {
  mediaPreference(true);
  const launcher = document.createElement('button');
  document.body.append(launcher);
  launcher.focus();
  const view = render(<Surface />);
  const nextDialog = document.createElement('dialog');
  const nextControl = document.createElement('button');
  nextDialog.append(nextControl);
  document.body.append(nextDialog);
  nextDialog.showModal();
  nextControl.focus();
  view.unmount();
  expect(nextControl).toHaveFocus();
  nextDialog.close();
  nextDialog.remove();
  launcher.remove();
});
