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
