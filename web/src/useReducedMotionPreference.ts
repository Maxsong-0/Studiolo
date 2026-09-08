import { useSyncExternalStore } from 'react';

const query = '(prefers-reduced-motion: reduce)';

function subscribe(notify: () => void) {
  const preference = window.matchMedia?.(query);
  preference?.addEventListener('change', notify);
  return () => preference?.removeEventListener('change', notify);
}

function snapshot() {
  return window.matchMedia?.(query).matches ?? true;
}

/** 与弹窗一致：实时订阅系统偏好，无查询能力时静态呈现。 */
export function useReducedMotionPreference() {
  return useSyncExternalStore(subscribe, snapshot, () => true);
}
