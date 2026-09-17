import { byteLength } from './canonicalModel';
import { isRecord, isHex } from './fileOperationModel';

export const validFileBytes = (value: unknown): value is number =>
  typeof value === 'number' &&
  Number.isSafeInteger(value) &&
  value >= 0 &&
  value <= 268_435_456;
export const validFilePath = (value: unknown): value is string =>
  typeof value === 'string' &&
  value.length > 0 &&
  byteLength(value) <= 4096 &&
  value.normalize('NFC') === value &&
  !/[\\\p{Cc}\uD800-\uDFFF]/u.test(value) &&
  value
    .split('/')
    .every((part) => part !== '' && part !== '.' && part !== '..');
export const validFilePresence = (value: unknown) =>
  isRecord(value) &&
  ((value.kind === 'missing' && Object.keys(value).length === 1) ||
    (value.kind === 'present' &&
      Object.keys(value).length === 4 &&
      isHex(value.revision) &&
      isHex(value.sha256) &&
      validFileBytes(value.bytes)));
