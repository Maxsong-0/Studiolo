import { StrictMode } from 'react';
import { renderHook } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useReducedMotionPreference } from '../useReducedMotionPreference';
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
