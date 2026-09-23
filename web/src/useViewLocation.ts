import { useCallback, useSyncExternalStore } from 'react';
import type { View } from './types';
import {
  defaultViewLocation,
  formatViewLocation,
  parseViewLocation,
  sameViewLocation,
} from './viewLocation';
import type { ViewLocation, ViewSlot } from './viewLocation';

const listeners = new Set<() => void>();
let readHash: string | null = null;
let parsed: ViewLocation = defaultViewLocation;

function announce() {
  listeners.forEach((listener) => listener());
}

/** hash 不变必须返回同一个对象：useSyncExternalStore 以引用判断是否重渲染。 */
function snapshot(): ViewLocation {
  const hash = window.location.hash;
  if (hash !== readHash) {
    readHash = hash;
    parsed = parseViewLocation(hash);
  }
  return parsed;
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    window.addEventListener('hashchange', announce);
    window.addEventListener('popstate', announce);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      window.removeEventListener('hashchange', announce);
      window.removeEventListener('popstate', announce);
    }
  };
}

/** 写位置用 replaceState：位置是「回到离开处」的记号，不该把后退键变成视图历史。 */
function write(next: ViewLocation) {
  if (sameViewLocation(snapshot(), next)) return;
  const hash = formatViewLocation(next);
  const target = `${window.location.pathname}${window.location.search}${
    hash ? `#${hash}` : ''
  }`;
  try {
    window.history.replaceState(window.history.state, '', target);
  } catch {
    // 少数环境禁用 History API：退回直接改 hash，位置照样留得住。
    window.location.hash = hash;
  }
  announce();
}

/**
 * 视图位置的唯一读写入口。位置活在 URL 里而不是组件内存里，
 * 所以会话重新核对（页面隐藏／恢复焦点）把工作台整棵子树重挂之后，
 * 各视图仍能把自己那一格原样读回来，刷新与分享同一链接也落在同一处。
 */
export function useViewLocation() {
  const location = useSyncExternalStore(subscribe, snapshot);
  const selectView = useCallback((view: View) => {
    const current = snapshot();
    if (view === current.view) return;
    // 换视图就清掉上一视图的项目／面板／条目：这些键按视图解释，串视图会读成别人的位置。
    write({ ...defaultViewLocation, view });
  }, []);
  // 一次写一组格子：换项目要同时清掉属于旧项目的条目，不该先写出一个错搭配的中间态。
  const select = useCallback(
    (patch: Partial<Record<ViewSlot, string | null>>) => {
      write({ ...snapshot(), ...patch });
    },
    [],
  );
  // 跨视图直达某一格（如从驾驶舱点项目直接进它的运作）：视图与格子一次写好，
  // 不先切视图再补格子，也不带上一视图的格子。
  const goTo = useCallback(
    (target: { view: View } & Partial<Record<ViewSlot, string | null>>) => {
      write({ ...defaultViewLocation, ...target });
    },
    [],
  );
  return { location, selectView, select, goTo };
}
