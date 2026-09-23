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

/**
 * 过渡只有 120ms（规范第 11 节），机器一忙，真实时钟下一帧就可能直接跳到终点，看不到中间帧。
 * 这里把动画读的时钟（performance.now）钉住，由测试手动拨：帧照常跑，每帧读到的时间由测试决定。
 * 单次拨动不超过 40ms：动画在下一帧才解析起点，间隔超过 40ms 时会改从解析那一刻起算。
 */
function manualClock() {
  let now = performance.now();
  vi.spyOn(performance, 'now').mockImplementation(() => now);
  return {
    advance(ms: number) {
      now += ms;
    },
    /** 每次检查拨快 10ms，直到断言成立。 */
    async runUntil(assertion: () => void) {
      await waitFor(
        () => {
          now += 10;
          assertion();
        },
        { interval: 5, timeout: 1500 },
      );
    },
  };
}

/** 过渡走到半路：透明度介于 0 与 1 之间，位移在 0 与 4px 之间（未开减少动效时）。 */
function midway(dialog: HTMLElement) {
  const opacity = Number(dialog.style.opacity);
  expect(opacity).toBeGreaterThan(0);
  expect(opacity).toBeLessThan(1);
  expect(dialog.style.transform).toMatch(/^translateY\([\d.]+px\)$/);
  const offset = Number(/[\d.]+/.exec(dialog.style.transform)?.[0]);
  expect(offset).toBeGreaterThan(0);
  expect(offset).toBeLessThan(4);
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
  const clock = manualClock();
  const close = vi.fn();
  render(<Surface onClose={close} />);
  const dialog = surface();
  expect(dialog).toHaveAttribute('open');
  // 入场起点：全透明，在原位下方 4px（规范第 11 节：120ms 透明度＋4px 位移）
  expect(dialog.style.opacity).toBe('0');
  expect(dialog.style.transform).toBe('translateY(4px)');
  clock.advance(30);
  await waitFor(() => midway(dialog), { interval: 5 });
  atRenderedFrame(() => {
    const visible = dialog.style.opacity;
    fireEvent.click(screen.getByRole('button', { name: '关闭' }));
    expect(dialog.style.opacity).toBe(visible);
  });
  expect(close).not.toHaveBeenCalled();
  cancel(dialog);
  await clock.runUntil(() => expect(close).toHaveBeenCalledTimes(1));
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
    const clock = manualClock();
    const animations = vi.mocked(motion.animate);
    const close = vi.fn();
    render(<Surface onClose={close} />);
    const dialog = surface();
    await clock.runUntil(() => expect(dialog.style.opacity).toBe('1'));
    fireEvent.click(screen.getByRole('button', { name: '关闭' }));
    const oldCompletion = animations.mock.calls[1][2]?.onComplete;
    expect(oldCompletion).toBeTypeOf('function');
    clock.advance(30);
    await waitFor(() => expect(Number(dialog.style.opacity)).toBeLessThan(1), {
      interval: 5,
    });
    midway(dialog);
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
    await clock.runUntil(() => expect(dialog.style.opacity).toBe('1'));
    expect(close).not.toHaveBeenCalled();
    cancel(dialog);
    await clock.runUntil(() => expect(close).toHaveBeenCalledTimes(1));
  },
);

it('输入可取消退出，减少动效立即移除位移，下一次关闭只淡出', async () => {
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
  await waitFor(
    () => {
      expect(dialog.style.transform).toBe('none');
      expect(close).toHaveBeenCalledTimes(1);
    },
    { timeout: 1500 },
  );
});

it('退场中启用减少效果立即去掉位移，淡出后关闭且回调采用最新值', async () => {
  const preference = mediaPreference(false);
  const clock = manualClock();
  const oldClose = vi.fn();
  const close = vi.fn();
  const view = render(<Surface onClose={oldClose} />);
  const dialog = surface();
  await clock.runUntil(() => expect(dialog.style.opacity).toBe('1'));
  view.rerender(<Surface onClose={close} />);
  cancel(dialog);
  clock.advance(30);
  await waitFor(() => midway(dialog), { interval: 5 });
  preference.change(true);
  expect(dialog.style.transform).toBe('none');
  expect(Number(dialog.style.opacity)).toBeLessThan(1);
  await clock.runUntil(() => expect(close).toHaveBeenCalledTimes(1));
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
  // 先完整出现再取消：退场要真的在走（还没画出第一帧就取消没有可收起的距离，会当场关闭）
  await entered();
  const dialog = surface();
  cancel(dialog);
  expect(close).not.toHaveBeenCalled();
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
  // 还没画出第一帧就取消：没有可收起的距离，当场关闭
  cancel(dialog);
  expect(close).toHaveBeenCalledTimes(1);
});

it('减少动效时只淡入淡出、全程没有位移', async () => {
  mediaPreference(true);
  const clock = manualClock();
  const close = vi.fn();
  render(<Surface onClose={close} />);
  const dialog = surface();
  expect(dialog.style.opacity).toBe('0');
  expect(dialog.style.transform).toBe('none');
  clock.advance(30);
  await waitFor(() => {
    const opacity = Number(dialog.style.opacity);
    expect(opacity).toBeGreaterThan(0);
    expect(opacity).toBeLessThan(1);
  });
  expect(dialog.style.transform).toBe('none');
  await clock.runUntil(() => expect(dialog.style.opacity).toBe('1'));
  cancel(dialog);
  expect(close).not.toHaveBeenCalled();
  clock.advance(30);
  await waitFor(() => expect(Number(dialog.style.opacity)).toBeLessThan(1));
  expect(dialog.style.transform).toBe('none');
  await clock.runUntil(() => expect(close).toHaveBeenCalledTimes(1));
});

it('关闭减少效果后不重放入场，后续关闭恢复可中断的位移动画', async () => {
  const preference = mediaPreference(true);
  const clock = manualClock();
  const close = vi.fn();
  render(<Surface onClose={close} />);
  const dialog = surface();
  await clock.runUntil(() => expect(dialog.style.opacity).toBe('1'));
  preference.change(false);
  expect(dialog.style.opacity).toBe('1');
  // 静止在完全显示时不留变换（恒等位移也会建层叠上下文）
  expect(dialog.style.transform).toBe('none');
  cancel(dialog);
  expect(close).not.toHaveBeenCalled();
  clock.advance(30);
  await waitFor(() => midway(dialog), { interval: 5 });
  await clock.runUntil(() => expect(close).toHaveBeenCalledTimes(1));
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
