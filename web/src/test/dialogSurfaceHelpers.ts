import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { expect, vi } from 'vitest';
import * as motion from 'motion';

/** 模拟可动态更新的系统动态效果偏好，不替代真实动画。 */
export function mediaPreference(reduced: boolean) {
  const preference = new EventTarget();
  Object.defineProperty(preference, 'matches', {
    configurable: true,
    value: reduced,
  });
  const remove = vi.spyOn(preference, 'removeEventListener');
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => preference),
  );
  return {
    remove,
    change(value: boolean) {
      Object.defineProperty(preference, 'matches', { value });
      const event = new Event('change');
      Object.defineProperty(event, 'matches', { value });
      act(() => {
        preference.dispatchEvent(event);
      });
    },
  };
}

export function surface() {
  return screen.getByRole('dialog', { name: '测试弹窗' });
}

export async function entered() {
  await waitFor(() => expect(surface().style.opacity).toBe('1'), {
    timeout: 1500,
  });
}

export function cancel(dialog: HTMLElement) {
  fireEvent(dialog, new Event('cancel', { cancelable: true }));
}

/**
 * 浏览器的关闭请求（返回手势、requestClose()、没被 keydown 拦下的 Esc）：先向对话框发 cancel，
 * 被拦下就到此为止；否则浏览器直接关掉对话框、再发 close。jsdom 两步都不做，这里按规范补上。
 * cancelable=false 即 CloseWatcher 判定「拦下一次之后没有新的用户激活」：preventDefault 无效，页面拦不住。
 */
export function closeRequest(dialog: HTMLElement, cancelable: boolean) {
  if (!fireEvent(dialog, new Event('cancel', { cancelable }))) return;
  nativeClose(dialog);
}

/** 浏览器自己关掉对话框：去掉 open，随后发 close（真实浏览器里 close 排在稍后的任务里）。 */
export function nativeClose(dialog: HTMLElement) {
  act(() => {
    (dialog as HTMLDialogElement).close();
    dialog.dispatchEvent(new Event('close'));
  });
}

export function atRenderedFrame(interact: () => void) {
  // stop 会补采样事件时刻；固定到已呈现帧，严格验证反向未重置端点。
  const clock = vi
    .spyOn(motion.time, 'now')
    .mockReturnValue(motion.frameData.timestamp);
  try {
    interact();
  } finally {
    clock.mockRestore();
  }
}
