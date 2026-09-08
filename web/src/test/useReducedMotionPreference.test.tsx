import { StrictMode } from 'react';
import { fireEvent, render, renderHook, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useReducedMotionPreference } from '../useReducedMotionPreference';
import { PointerSurface } from '../components/PointerSurface';
import { mediaPreference } from './dialogSurfaceHelpers';

it('系统偏好开启与关闭后都立即更新，无需卸载组件', () => {
  const preference = mediaPreference(true);
  const { result } = renderHook(useReducedMotionPreference);
  expect(result.current).toBe(true);
  preference.change(false);
  expect(result.current).toBe(false);
  preference.change(true);
  expect(result.current).toBe(true);
});

it('无偏好查询能力时保守采用静态呈现', () => {
  vi.stubGlobal('matchMedia', undefined);
  const { result } = renderHook(useReducedMotionPreference);
  expect(result.current).toBe(true);
});

it('StrictMode 重建订阅后仍可响应，并在卸载时移除监听', () => {
  const preference = mediaPreference(false);
  const { result, unmount } = renderHook(useReducedMotionPreference, {
    wrapper: StrictMode,
  });
  preference.change(true);
  expect(result.current).toBe(true);
  const removals = preference.remove.mock.calls.length;
  unmount();
  expect(preference.remove).toHaveBeenCalledTimes(removals + 1);
  expect(preference.remove).toHaveBeenLastCalledWith(
    'change',
    expect.any(Function),
  );
});

function moveMouse(surface: Element) {
  const event = new Event('pointermove', { bubbles: true });
  Object.assign(event, { pointerType: 'mouse', clientX: 100, clientY: 80 });
  fireEvent(surface, event);
}

it('指针高光随偏好恢复，开启减少动态立即清空，关闭后等待新指针动作', async () => {
  const preference = mediaPreference(true);
  const { container } = render(<PointerSurface>项目内容</PointerSurface>);
  const surface = container.querySelector('.pointer-surface')!;
  const light = container.querySelector<HTMLElement>('.pointer-light')!;
  expect(light.style.opacity).toBe('0');

  preference.change(false);
  moveMouse(surface);
  await waitFor(() => expect(Number(light.style.opacity)).toBeGreaterThan(0.1));

  preference.change(true);
  await waitFor(() => expect(light.style.opacity).toBe('0'));
  moveMouse(surface);
  preference.change(false);
  expect(light.style.opacity).toBe('0');

  moveMouse(surface);
  await waitFor(() => expect(Number(light.style.opacity)).toBeGreaterThan(0.1));
});
