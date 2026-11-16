/**
 * 能力集各字段的人话名字（T-198）：照后端 `crates/policy/src/model.rs` 里 CapabilitySet、ReadCaps、
 * WriteCaps、PolicyCaps、HardBinding 的注释写。纯数据；顺序就是差异表里的行序。
 * 表里没有的字段（后端以后加的、或写错的）差异表照样列出，只是名字用原路径，不猜它的意思。
 */

export interface CapabilityField {
  /** 字段在正文里的路径，如 `read.project_whitelist`。 */
  readonly path: string;
  /** 界面上的人话。 */
  readonly label: string;
  /** 这一项管什么，一句话。 */
  readonly note: string;
}

export const CAPABILITY_FIELDS: readonly CapabilityField[] = [
  { path: 'ai_type', label: 'AI 类型', note: '必须和对象同名' },
  {
    path: 'read.project_whitelist',
    label: '可读的项目',
    note: '路径前缀名单，名单之外读不到',
  },
  {
    path: 'read.deny_by_default',
    label: '名单之外一律不可读',
    note: '必须是「是」',
  },
  {
    path: 'read.rules_whitelist',
    label: '可读的规则',
    note: '规则文件名单，空着就一条规则都读不到',
  },
  { path: 'write.board', label: '可写黑板', note: '只限下面列出的项目' },
  {
    path: 'write.board_projects',
    label: '可写黑板的项目',
    note: '必须都在可读的项目之内',
  },
  {
    path: 'write.content_patch_only',
    label: '改正文只能提交补丁',
    note: '基于原版本加补丁，人预览后才落地',
  },
  {
    path: 'write.content_direct',
    label: '可整页覆盖正文',
    note: '永远是「否」',
  },
  {
    path: 'policy.propose_desired',
    label: '可提规则草案',
    note: '提案永不自动生效',
  },
  { path: 'policy.activate', label: '可自行发布', note: '永远是「否」' },
  {
    path: 'hard_bound_skills',
    label: '硬绑定的 skill',
    note: '触发条件 → 必须用的 skill',
  },
];

const BY_PATH: ReadonlyMap<string, CapabilityField> = new Map(
  CAPABILITY_FIELDS.map((field) => [field.path, field]),
);

/** 字段的人话；表里没有就是 null（调用方照原路径显示）。 */
export function capabilityField(path: string): CapabilityField | null {
  return BY_PATH.get(path) ?? null;
}
