/**
 * 策略平面术语表（T-130 设计规范第 9 节）：界面一律先写人话，原术语用等宽小字跟在后面，
 * 例如「规则草案 desired」「生效中 effective」。标题、表头与字段说明可以带原术语；
 * 按钮的可访问名只用人话，不带原术语。纯数据，界面统一从这里取，不在各处另写一套叫法。
 */

export interface PolicyTerm {
  /** 界面上的人话。 */
  readonly human: string;
  /** 接口、契约、日志里的原术语。 */
  readonly raw: string;
}

export const POLICY_TERMS = {
  /** 待确认的意图：提交后永不自动生效，发布才成为新版本。 */
  desired: { human: '规则草案', raw: 'desired' },
  /** 当前正在约束各 AI 的那一份策略。 */
  effective: { human: '生效中', raw: 'effective' },
  /** 验证器给出的六位码，发布、回滚、解除全局撤销都要用它做二次认证。 */
  oneTimeCode: { human: '一次性验证码', raw: 'One-time Code' },
  /** 实例 / 类型 / 全局三级；撤销即刻生效。 */
  revoke: { human: '撤销接入', raw: 'revoke' },
  /** 发布类动作被服务端要求就地再认证一次。 */
  stepUp: { human: '二次认证', raw: 'step-up' },
  /** 规则文件名、skill 名或 AI 类型名。 */
  subject: { human: '对象', raw: 'subject' },
  /** 正文或整份策略的 SHA-256，界面只显示前 12 位。 */
  sha256: { human: '摘要', raw: 'sha256' },
  diff: { human: '差异', raw: 'diff' },
  validate: { human: '校验', raw: 'validate' },
  activate: { human: '发布', raw: 'activate' },
  rollback: { human: '回滚', raw: 'rollback' },
} as const satisfies Record<string, PolicyTerm>;

export type PolicyTermName = keyof typeof POLICY_TERMS;

/** 撤销接入的三级（叫法与 policyModel 的 LEVEL_LABEL 一致，这里附上原术语）。 */
export const REVOKE_LEVEL_TERMS = {
  instance: { human: '实例', raw: 'instance' },
  type: { human: '类型', raw: 'type' },
  global: { human: '全局', raw: 'global' },
} as const satisfies Record<string, PolicyTerm>;

/**
 * 校验报告里改动的方向：只有扩大用红色，收窄与未知一律中性色。
 * 接口的报告只分扩大项（expansions）与收窄项（narrowings），不单列未知项。
 */
export const CHANGE_TERMS = {
  expansion: { human: '扩大', raw: 'expansion' },
  narrowing: { human: '收窄', raw: 'narrowing' },
  unknown: { human: '未知', raw: 'unknown' },
} as const satisfies Record<string, PolicyTerm>;
