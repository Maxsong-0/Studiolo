/** 协作页的四个标签（设计规范第 7 节）。 */
export const COLLAB_TABS = [
  { id: 'topology', label: '拓扑' },
  { id: 'board', label: '任务与回执' },
  { id: 'timeline', label: '时间线' },
  { id: 'execution', label: '执行' },
] as const;

export type CollabPanel = (typeof COLLAB_TABS)[number]['id'];

/** 位置里的 `panel` 格对应哪个标签：默认面板拓扑记作 null；未知面板值一律当拓扑，不猜。 */
export function collabPanel(value: string | null): CollabPanel {
  return value === 'board' || value === 'timeline' || value === 'execution'
    ? value
    : 'topology';
}
