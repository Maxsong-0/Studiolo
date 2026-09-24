import { StrictMode, useRef } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useDialogSurface } from '../useDialogSurface';
import {
  closeRequest,
  entered,
  mediaPreference,
  nativeClose,
  surface,
} from './dialogSurfaceHelpers';

/*
 * T-170：浏览器绕过页面直接关掉对话框（Chrome CloseWatcher 下不可取消的 cancel，如返回手势连按）时，
 * useDialogSurface 跟着 close 事件收起：归还焦点、作废进行中的过渡与此前的异步交互、onClose 只回调一次。
 * 8 个对话框都经这个钩子呈现，同一根因在这里一并验。
 */

/** 每点一次「发起读取」记下一份交互凭证，与异步读取在发起时取凭证相同。 */
const interactions: Array<() => boolean> = [];

/** busy 时页面自己不收起（与认证在途的 PolicyStepUp 相同），只剩浏览器强制关闭这一条路。 */
function Surface({
  onClose,
  busy = false,
}: {
  onClose: () => void;
  busy?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const { dismiss, captureInteraction } = useDialogSurface(ref, onClose);
  return (
    <dialog
      ref={ref}
      aria-label="测试弹窗"
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) dismiss();
      }}
    >
      <button data-dialog-dismiss onClick={dismiss}>
        关闭
      </button>
      <button>继续操作</button>
      <button onClick={() => interactions.push(captureInteraction())}>
        发起读取
      </button>
    </dialog>
  );
}

async function wait(ms: number) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

it('页面不收起时被浏览器强制关闭：随之收起、归还焦点、作废此前的交互，onClose 只回调一次', () => {
  vi.stubGlobal('matchMedia', undefined);
  const launcher = document.createElement('button');
  document.body.append(launcher);
  launcher.focus();
  const close = vi.fn();
  const view = render(<Surface onClose={close} busy />);
  const dialog = surface();
  // 关掉后 jsdom 把未 open 的 dialog 当作隐藏，里面的按钮先取好。
  const dismissButton = screen.getByRole('button', { name: '关闭' });
  const read = screen.getByRole('button', { name: '发起读取' });
  fireEvent.click(read);
  const interaction = interactions.at(-1)!;
  read.focus();
  // 可取消的请求被页面拦下：什么都不发生。
  closeRequest(dialog, true);
  expect(dialog).toHaveAttribute('open');
  expect(close).not.toHaveBeenCalled();
  expect(interaction()).toBe(true);
  // 不可取消：浏览器直接关掉。
  closeRequest(dialog, false);
  expect(close).toHaveBeenCalledTimes(1);
  expect(dialog).not.toHaveAttribute('open');
  expect(launcher).toHaveFocus();
  expect(interaction()).toBe(false);
  fireEvent(dialog, new Event('close'));
  fireEvent.click(dismissButton);
  expect(close).toHaveBeenCalledTimes(1);
  view.unmount();
  launcher.remove();
});

it('自己收起之后浏览器补发的 close 不再回调', () => {
  vi.stubGlobal('matchMedia', undefined);
  const close = vi.fn();
  render(<Surface onClose={close} />);
  const dialog = surface();
  fireEvent.click(screen.getByRole('button', { name: '关闭' }));
  expect(close).toHaveBeenCalledTimes(1);
  fireEvent(dialog, new Event('close'));
  expect(close).toHaveBeenCalledTimes(1);
});

it('退场过渡途中被浏览器关掉：立即收起，旧过渡走完也不再回调', async () => {
  mediaPreference(false);
  const close = vi.fn();
  render(<Surface onClose={close} />);
  await entered();
  const dialog = surface();
  fireEvent.click(screen.getByRole('button', { name: '关闭' }));
  expect(close).not.toHaveBeenCalled();
  nativeClose(dialog);
  expect(close).toHaveBeenCalledTimes(1);
  await wait(300);
  expect(close).toHaveBeenCalledTimes(1);
  expect(dialog).not.toHaveAttribute('open');
});

it('入场过渡途中被浏览器关掉：立即收起，之后再点关闭也不重复回调', async () => {
  mediaPreference(false);
  const close = vi.fn();
  render(<Surface onClose={close} />);
  const dialog = surface();
  const dismissButton = screen.getByRole('button', { name: '关闭' });
  expect(dialog.style.opacity).toBe('0');
  nativeClose(dialog);
  expect(close).toHaveBeenCalledTimes(1);
  await wait(300);
  fireEvent.click(dismissButton);
  expect(close).toHaveBeenCalledTimes(1);
  expect(dialog).not.toHaveAttribute('open');
});

it('严格模式重挂后，前一轮清理排下的 close 在对话框开着时才送到：不当作关闭', () => {
  vi.stubGlobal('matchMedia', undefined);
  const close = vi.fn();
  render(
    <StrictMode>
      <Surface onClose={close} busy />
    </StrictMode>,
  );
  const dialog = surface();
  expect(dialog).toHaveAttribute('open');
  fireEvent(dialog, new Event('close'));
  expect(dialog).toHaveAttribute('open');
  expect(close).not.toHaveBeenCalled();
  closeRequest(dialog, false);
  expect(close).toHaveBeenCalledTimes(1);
});

it('卸载之后送到的 close 不回调', () => {
  vi.stubGlobal('matchMedia', undefined);
  const close = vi.fn();
  const view = render(<Surface onClose={close} />);
  const dialog = surface();
  view.unmount();
  dialog.dispatchEvent(new Event('close'));
  expect(close).not.toHaveBeenCalled();
});
