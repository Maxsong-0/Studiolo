import { ApiError, getJson } from './sessionHttp';
import {
  activateOutcome,
  desiredEnvelope,
  desiredList,
  policyStatus,
  revokeOutcome,
  shortId,
  versionList,
  versionRecord,
} from './policyModel';
import type {
  ActivateRequest,
  DesiredInput,
  RevokeRequest,
} from './policyModel';

export const STATUS = '/api/v1/policy';
export const DESIRED = '/api/v1/policy/desired';
export const VALIDATE = '/api/v1/policy/validate';
export const WITHDRAW = '/api/v1/policy/withdraw';
export const ACTIVATE = '/api/v1/policy/activate';
export const VERSIONS = '/api/v1/policy/versions';
export const REVOKE = '/api/v1/agents/revoke';

type Writer = (
  path: string,
  body: unknown,
  signal: AbortSignal,
) => Promise<unknown>;
let writer: Writer | null = null;

/** 仅 SessionBoundary 可装配写者；没有直接 fetch 的写入回退。 */
export function installPolicyWriter(next: Writer) {
  writer = next;
  return () => {
    if (writer === next) writer = null;
  };
}

function post(path: string, body: unknown, signal: AbortSignal) {
  if (!writer) throw new ApiError('尚未装配受控会话写入，请重新登录。', 403);
  return writer(path, body, signal);
}

function codeOf(error: unknown): string | undefined {
  if (
    !(error instanceof ApiError) ||
    !error.details ||
    typeof error.details !== 'object'
  )
    return undefined;
  const code = (error.details as { code?: unknown }).code;
  return typeof code === 'string' ? code : undefined;
}

/** 409 `newer_desired_same_submitter` 带回的、挡住发布的更新草案号；字段缺失或形状不对就当没有，只少一句提示。 */
function newerDesiredIds(error: ApiError): string[] {
  if (!error.details || typeof error.details !== 'object') return [];
  const ids = (error.details as { newer_desired_ids?: unknown })
    .newer_desired_ids;
  return Array.isArray(ids)
    ? ids.filter((id): id is string => typeof id === 'string')
    : [];
}

/** 404 `policy_plane_absent` 表示本入口没装策略平面，界面按「未接入」显示而不是报错。 */
export function isAbsent(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    error.status === 404 &&
    codeOf(error) === 'policy_plane_absent'
  );
}

/**
 * 403 `reauthentication_required`：服务端已记下 intent 并拒绝，发布**没有**发生，只差一次
 * One-time Code。界面据此就地弹认证窗口；认证由人完成、窗口由服务端裁定，前端不缓存结果。
 */
export function needsReauthentication(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    error.status === 403 &&
    codeOf(error) === 'reauthentication_required'
  );
}

export async function readPolicyStatus(signal: AbortSignal) {
  return policyStatus(await getJson(STATUS, signal));
}
export async function readDesired(signal: AbortSignal) {
  return desiredList(await getJson(DESIRED, signal));
}
export async function readVersions(signal: AbortSignal) {
  return versionList(await getJson(VERSIONS, signal));
}
// 共用认证入口只有两个阻塞许可；整页读取顺序执行，避免三请求自触发503。
// 任一步失败或取消都停止，不重放发布，也不继续发送剩余读取。
export async function readPolicySnapshot(signal: AbortSignal) {
  const status = await readPolicyStatus(signal);
  const desired = await readDesired(signal);
  const versions = await readVersions(signal);
  return { status, desired, versions };
}

export async function readVersion(version: number, signal: AbortSignal) {
  return versionRecord(await getJson(`${VERSIONS}/${version}`, signal));
}
export async function submitDesired(input: DesiredInput, signal: AbortSignal) {
  return desiredEnvelope(await post(DESIRED, input, signal));
}
export async function validateDesired(desired_id: string, signal: AbortSignal) {
  return desiredEnvelope(await post(VALIDATE, { desired_id }, signal));
}
export async function withdrawDesired(desired_id: string, signal: AbortSignal) {
  return desiredEnvelope(await post(WITHDRAW, { desired_id }, signal));
}
export async function activatePolicy(
  request: ActivateRequest,
  signal: AbortSignal,
) {
  return activateOutcome(await post(ACTIVATE, request, signal));
}
export async function revokeAgent(request: RevokeRequest, signal: AbortSignal) {
  return revokeOutcome(await post(REVOKE, request, signal));
}

/** 把发布失败翻译成人能行动的一句话；403 需要二次认证，409 需要重新校验或先处理更新的草案。 */
export function activateFailure(error: unknown): string {
  if (!(error instanceof ApiError)) return '发布未完成，请核对当前 effective。';
  const code = codeOf(error);
  if (code === 'reauthentication_required')
    return '发布需要完成二次认证（One-time Code）后才会生效。';
  if (error.status === 403)
    return '发布只接受可信入口的请求；当前入口不能发布。';
  if (error.status === 409 && code === 'hash_mismatch')
    return 'effective 已变化：请刷新并重新校验后再发布。';
  if (error.status === 409 && code === 'newer_desired_same_submitter') {
    const newer = newerDesiredIds(error).map(shortId).join('、');
    return `没有发布：同一提交者还有更新的未发布草案${newer ? `（${newer}）` : ''}，只能发布其中最新的那一份；要发这份，先撤回更新的。`;
  }
  if (error.status === 409 && code === 'no_l3_path')
    return '本入口未启用二次认证，没有 One-time Code 路径，不能发布。';
  if (error.status === 422) return '校验未通过，请查看该 desired 的逐条问题。';
  if (error.status === 401) return '登录已失效，请重新登录。';
  return error.message;
}
