import type { DocumentNavigationData } from './types';

const record = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object';
const location = (value: unknown) =>
  record(value) &&
  typeof value.path === 'string' &&
  Number.isSafeInteger(value.line) &&
  Number(value.line) > 0;
const statuses = new Set([
  'resolved',
  'ambiguous',
  'missing',
  'missing_heading',
  'ambiguous_heading',
]);

export function validNavigation(
  value: unknown,
): value is DocumentNavigationData {
  return (
    record(value) &&
    typeof value.truncated === 'boolean' &&
    Array.isArray(value.outline) &&
    value.outline.every(
      (entry: unknown) =>
        record(entry) &&
        typeof entry.title === 'string' &&
        Number.isInteger(entry.level) &&
        Number(entry.level) >= 1 &&
        Number(entry.level) <= 6 &&
        location(entry.source),
    ) &&
    Array.isArray(value.outgoing) &&
    value.outgoing.every(
      (entry: unknown) =>
        record(entry) &&
        typeof entry.label === 'string' &&
        typeof entry.raw === 'string' &&
        location(entry.source) &&
        typeof entry.status === 'string' &&
        statuses.has(entry.status) &&
        Array.isArray(entry.targets) &&
        entry.targets.every(
          (target: unknown) =>
            location(target) &&
            record(target) &&
            typeof target.title === 'string',
        ) &&
        (entry.status !== 'resolved' || entry.targets.length === 1),
    ) &&
    Array.isArray(value.backlinks) &&
    value.backlinks.every(
      (entry: unknown) =>
        record(entry) &&
        typeof entry.title === 'string' &&
        typeof entry.context === 'string' &&
        location(entry.source),
    )
  );
}
