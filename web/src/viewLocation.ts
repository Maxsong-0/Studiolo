import type { View } from './types';

/**
 * 视图位置：只放「在看哪个视图／哪个项目／哪个面板」这类非敏感导航状态。
 * 绝不放会话凭据、CSRF、因子状态、搜索词或笔记正文——这份状态会进 URL 与浏览历史。
 */
export interface ViewLocation {
  view: View;
  /** 项目 root：黑板、执行与运作三个视图的项目选择。 */
  project: string | null;
  /** 视图内的面板：如黑板的 `replay`、文档的 `tree`。 */
  panel: string | null;
  /** 面板内选中的条目：如执行任务 id。 */
  item: string | null;
}

export type ViewSlot = 'project' | 'panel' | 'item';

const views: readonly View[] = [
  'overview',
  'inbox',
  'documents',
  'operations',
  'policy',
  'board',
  'execution',
];
const slots: readonly ViewSlot[] = ['project', 'panel', 'item'];

export const defaultViewLocation: ViewLocation = {
  view: 'overview',
  project: null,
  panel: null,
  item: null,
};

/** 单格上限：位置只存短标识，长内容一律当作不是位置。 */
const slotLimit = 512;
const control = /[\p{Cc}]/u;

function isView(value: string | null): value is View {
  return value !== null && (views as readonly string[]).includes(value);
}

/** URL 是外部输入：空值、超长值与控制字符都不算位置，一律丢弃而不是原样回显。 */
function slotValue(value: string | null): string | null {
  if (value === null) return null;
  const trimmed = value.trim();
  if (!trimmed || trimmed.length > slotLimit) return null;
  return control.test(trimmed) ? null : trimmed;
}

/** 解析 hash 中的视图位置；未知视图或损坏内容回落默认视图，绝不抛错。 */
export function parseViewLocation(hash: string): ViewLocation {
  const params = new URLSearchParams(hash.replace(/^#/, ''));
  const view = params.get('view');
  if (!isView(view)) return defaultViewLocation;
  return {
    view,
    project: slotValue(params.get('project')),
    panel: slotValue(params.get('panel')),
    item: slotValue(params.get('item')),
  };
}

/** 序列化为 hash（不含 `#`）；默认位置序列化成空串，首屏不给 URL 添噪。 */
export function formatViewLocation(location: ViewLocation): string {
  const params = new URLSearchParams();
  params.set('view', location.view);
  for (const slot of slots) {
    const value = slotValue(location[slot]);
    if (value) params.set(slot, value);
  }
  const encoded = params.toString();
  return encoded === `view=${defaultViewLocation.view}` ? '' : encoded;
}

export function sameViewLocation(left: ViewLocation, right: ViewLocation) {
  return (
    left.view === right.view &&
    slots.every((slot) => left[slot] === right[slot])
  );
}
