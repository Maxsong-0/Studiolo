import { ApiError, getJson } from '../sessionHttp';
import { parseAgents } from './model';
export async function readAgents(signal: AbortSignal) {
  return parseAgents(await getJson('/api/v1/agents', signal));
}
export function agentsAbsent(error: unknown): boolean {
  if (
    !(error instanceof ApiError) ||
    error.status !== 404 ||
    !error.details ||
    typeof error.details !== 'object'
  )
    return false;
  return ['agents_plane_absent', 'policy_plane_absent'].includes(
    String((error.details as { code?: unknown }).code),
  );
}
