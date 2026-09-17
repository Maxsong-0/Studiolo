import {
  invalidFileOperation as invalid,
  isHex,
  isRecord,
} from './fileOperationModel';
import type {
  FileOperationReceipt,
  FileRenameRequest,
} from './fileOperationModel';
import { validFilePath, validFilePresence } from './fileOperationPrimitives';

const keys = (value: Record<string, unknown>, expected: string[]) =>
  Object.keys(value).length === expected.length &&
  expected.every((key) => Object.hasOwn(value, key));
export const validU64 = (value: unknown): value is string =>
  typeof value === 'string' &&
  /^(0|[1-9][0-9]{0,19})$/.test(value) &&
  (value.length < 20 || value <= '18446744073709551615');
/** 输入辅助校验不是服务器的整文件授权；目标字串不 trim/改名/规范化。 */
export function renameTargetError(from: string, to: string): string | null {
  if (
    ![from, to].every(validFilePath) ||
    !to.endsWith('.md') ||
    !from.endsWith('.md') ||
    [from, to].some(
      (path) =>
        /[\p{Cf}\p{Zl}\p{Zp}]/u.test(path) ||
        /^[a-z]:/i.test(path) ||
        path.split('/').some((part) => part.startsWith('.')),
    )
  )
    return '请输入规范的 Vault 相对 Markdown 路径（精确 .md），目标父目录须已存在。';
  const a = from.toLowerCase();
  const b = to.toLowerCase();
  if (a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`))
    return '源与目标不能相同、使用大小写别名或互为父路径。';
  return null;
}
/** 只验 wire 可证明的不变量；前端没有正文 hash 重算或真实执行权。 */
export function renameReceipt(
  value: unknown,
  request: FileRenameRequest,
): FileOperationReceipt {
  if (
    renameTargetError(request.from, request.to) ||
    !isRecord(value) ||
    !keys(value, ['status', 'receipt']) ||
    value.status !== 'committed' ||
    !isRecord(value.receipt)
  )
    invalid();
  const r = value.receipt;
  if (
    !keys(r, [
      'operation_id',
      'sequence',
      'outcome',
      'effects',
      'tombstone_id',
      'evidence_id',
    ]) ||
    !isHex(r.operation_id) ||
    !validU64(r.sequence) ||
    r.sequence === '0' ||
    r.outcome !== 'committed' ||
    r.tombstone_id !== r.operation_id ||
    r.evidence_id !== null ||
    !Array.isArray(r.effects) ||
    r.effects.length !== 2
  )
    invalid();
  for (const [index, path] of [request.to, request.from].entries()) {
    const effect: unknown = r.effects[index];
    if (
      !isRecord(effect) ||
      !keys(effect, ['kind', 'path', 'before', 'after']) ||
      effect.kind !== 'confirmed' ||
      effect.path !== path ||
      !validFilePresence(effect.before) ||
      !validFilePresence(effect.after)
    )
      invalid();
  }
  const receipt = r as unknown as FileOperationReceipt;
  const [target, source] = receipt.effects;
  if (
    !source ||
    target.before.kind !== 'missing' ||
    target.after.kind !== 'present' ||
    source.before.kind !== 'present' ||
    source.after.kind !== 'missing' ||
    source.before.revision !== request.expected_revision ||
    target.after.revision === request.expected_revision ||
    target.after.sha256 !== source.before.sha256 ||
    target.after.bytes !== source.before.bytes
  )
    invalid();
  return receipt;
}
