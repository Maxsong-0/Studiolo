import type { View } from './types';

/**
 * 视图位置：只放「在看哪个视图／哪个项目／哪个面板」这类非敏感导航状态。
 * 绝不放会话凭据、CSRF、因子状态、搜索词或笔记正文——这份状态会进 URL 与浏览历史。
 */
export interface ViewLocation {
  view: View;
  /** 项目 root：项目运作与协作各标签的项目选择。 */
  project: string | null;
  /**
   * 视图内的面板：如项目的 `operations`、协作的 `timeline`、文档的 `tree`、规则的 `edit`／`access`；
   * 默认面板记作 null。
   */
  panel: string | null;
  /** 面板内选中的条目：如执行任务 id、策略版本号、在编辑的规则名（subject）。 */
  item: string | null;
}

export type ViewSlot = 'project' | 'panel' | 'item';

/** 顶栏六个视图，顺序与导航一致（设计规范第 4 节）。 */
const views: readonly View[] = [
  'cockpit',
  'projects',
  'documents',
  'collab',
  'rules',
  'policy',
];
const slots: readonly ViewSlot[] = ['project', 'panel', 'item'];

export const defaultViewLocation: ViewLocation = {
  view: 'cockpit',
  project: null,
  panel: null,
  item: null,
};

/** 单格上限：位置只存短标识，长内容一律当作不是位置。 */
const slotLimit = 512;
const control = /[\p{Cc}]/u;

type Slots = Omit<ViewLocation, 'view'>;

/**
 * 旧七视图（T-130 之前）的链接照样能打开：按设计规范第 4 节「旧链接迁移」表换算成新位置。
 * 只在解析时迁移，写回 URL 一律用新名；旧视图没用过的格子不带过去，免得被新视图读成别的意思。
 */
const legacyViews: Record<string, (slots: Slots) => ViewLocation> = {
  overview: () => defaultViewLocation,
  // 待我动作并进驾驶舱：驾驶舱首格就是待我动作。
  inbox: () => defaultViewLocation,
  // 旧运作视图的 panel 值原样放进 item，留给运作页内部细分用。
  operations: ({ project, panel }) => ({
    view: 'projects',
    project,
    panel: 'operations',
    item: panel,
  }),
  board: ({ project, panel }) => ({
    view: 'collab',
    project,
    panel: panel === 'replay' ? 'timeline' : 'board',
    item: null,
  }),
  execution: ({ project, item }) => ({
    view: 'collab',
    project,
    panel: 'execution',
    item,
  }),
};

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

/** 解析 hash 中的视图位置；旧视图名先迁移，未知视图或损坏内容回落默认视图，绝不抛错。 */
export function parseViewLocation(hash: string): ViewLocation {
  const params = new URLSearchParams(hash.replace(/^#/, ''));
  const view = params.get('view');
  const location: Slots = {
    project: slotValue(params.get('project')),
    panel: slotValue(params.get('panel')),
    item: slotValue(params.get('item')),
  };
  // hasOwn 而不是 in：`view=constructor` 这类原型链上的名字不能被当成旧视图。
  if (view !== null && Object.hasOwn(legacyViews, view))
    return legacyViews[view](location);
  if (!isView(view)) return defaultViewLocation;
  return { view, ...location };
}

/** 序列化为 hash（不含 `#`），只写新视图名；默认位置序列化成空串，首屏不给 URL 添噪。 */
export function formatViewLocation(location: ViewLocation): string {
  if (!isView(location.view)) return '';
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
