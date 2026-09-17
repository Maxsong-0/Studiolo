import type {
  FileOperationReceipt,
  FileRenameRequest,
} from './fileOperationModel';
import type { Workbench } from './types';
import { validFreshness } from './freshness';
import { validU64 } from './renameValidation';

export type RenameNavigation = (
  request: FileRenameRequest,
  receipt: FileOperationReceipt,
  signal: AbortSignal,
  isCurrent: () => boolean,
) => Promise<boolean>;
/** 使用原十进制值比较；不能以 JS Number 截断 u64 水位。 */
export function renameSnapshotReady(
  snapshot: Workbench | null,
  sequence: string,
): boolean {
  const freshness = snapshot?.freshness;
  if (
    !validU64(sequence) ||
    sequence === '0' ||
    !validFreshness(freshness) ||
    !freshness ||
    freshness.stale
  )
    return false;
  return [freshness.indexed_through, freshness.committed_through].every(
    (watermark) => validU64(watermark) && BigInt(watermark) >= BigInt(sequence),
  );
}
