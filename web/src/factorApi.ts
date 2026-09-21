import type { FactorAction, FactorOutcome, FactorState } from './factorModel';

/** 会话拥有者提供的二次认证入口：读当前安全状态、提交一次 One-time Code。 */
export type FactorRuntime = {
  state: (signal: AbortSignal) => Promise<FactorState | null>;
  submit: (
    action: FactorAction,
    signal?: AbortSignal,
  ) => Promise<FactorOutcome>;
};

let runtime: FactorRuntime | null = null;

/**
 * 仅 SessionBoundary 可装配二次认证入口；没有直接 fetch 的回退。
 *
 * 装配的是同一条受控会话的方法，界面因此不持有凭据、不自行判断认证是否有效——
 * 有效期始终由服务端的近期认证窗口裁定。
 */
export function installFactorRuntime(next: FactorRuntime) {
  runtime = next;
  return () => {
    if (runtime === next) runtime = null;
  };
}

/** 未装配即没有就地认证入口；调用方据此显示「请重新登录」而不是假装可以认证。 */
export function factorRuntime(): FactorRuntime | null {
  return runtime;
}
