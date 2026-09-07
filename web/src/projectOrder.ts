import type { Project } from './types';

// 与后端闭集（crates/workbench/src/derive.rs 的生命周期六值）逐字一致；闭集外的值后端已拒绝，
// 只会以 null 到达前端，与「未建档」同档排在最后。
const lifecycleOrder: Record<string, number> = {
  活跃: 0,
  暂停: 1,
  已交付维护: 2,
  静态资料: 3,
  候选: 4,
  未建档: 5,
};
const names = new Intl.Collator('zh-CN', {
  numeric: true,
  sensitivity: 'base',
});
const latinInitial = /^[a-z]/i;
const rank = (project: Project) =>
  lifecycleOrder[project.lifecycle.value ?? ''] ?? 5;

function compareName(left: string, right: string) {
  const scriptOrder =
    Number(latinInitial.test(right)) - Number(latinInitial.test(left));
  return scriptOrder || names.compare(left, right);
}

export function sortProjects(projects: readonly Project[]): Project[] {
  return [...projects].sort((left, right) => {
    return (
      rank(left) - rank(right) ||
      compareName(left.name, right.name) ||
      names.compare(left.root, right.root)
    );
  });
}
