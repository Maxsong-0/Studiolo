import type { RustSourceCoverage, RustSourceLocation } from './rustCallSites';

export const record = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);
export const integer = (
  value: unknown,
  max: number,
  min = 0,
): value is number =>
  Number.isSafeInteger(value) && Number(value) >= min && Number(value) <= max;
const encoder = new TextEncoder();
const controls =
  /[\u00ad\u0600-\u0605\u061c\u06dd\u070f\u0890-\u0891\u08e2\u180e\u200b-\u200f\u2028-\u202e\u2060-\u2064\u2066-\u206f\ufeff\ufff9-\ufffb\u{110bd}\u{110cd}\u{13430}-\u{1343f}\u{1bca0}-\u{1bca3}\u{1d173}-\u{1d17a}\u{e0001}\u{e0020}-\u{e007f}]/u;
function hasCombiningMark(value: string): boolean {
  return [...value].some((character) => {
    const code = character.codePointAt(0)!;
    return [
      [0x0300, 0x036f],
      [0x1ab0, 0x1aff],
      [0x1dc0, 0x1dff],
      [0x20d0, 0x20ff],
      [0xfe20, 0xfe2f],
    ].some(([start, end]) => code >= start && code <= end);
  });
}
export function visibleText(value: unknown, limit: number): value is string {
  return (
    typeof value === 'string' &&
    value.trim().length > 0 &&
    value.length <= limit &&
    encoder.encode(value).length <= limit &&
    !controls.test(value) &&
    !/\p{Surrogate}/u.test(value) &&
    ![...value].some((character) => {
      const code = character.charCodeAt(0);
      return code < 32 || (code >= 127 && code <= 159);
    })
  );
}

export function rustPath(value: unknown): value is string {
  if (
    !visibleText(value, 1024) ||
    !value.endsWith('.rs') ||
    hasCombiningMark(value) ||
    value.normalize('NFC') !== value ||
    value.includes('\\') ||
    (value.charCodeAt(0) < 128 && value[1] === ':') ||
    /%(?:2f|5c|00|2e)/i.test(value)
  )
    return false;
  const parts = value.split('/');
  return (
    parts.length <= 16 &&
    parts.every((part) => part.length > 0 && !part.startsWith('.'))
  );
}

export function collisionFree(files: RustSourceCoverage[]): boolean {
  const prefixes = new Map<string, string>();
  const paths = new Set(files.map((file) => file.path));
  for (const file of files) {
    const parts = file.path.split('/');
    for (let index = 1; index <= parts.length; index += 1) {
      const path = parts.slice(0, index).join('/');
      if (index < parts.length && paths.has(path)) return false;
      const previous = prefixes.get(path.toLowerCase());
      if (previous !== undefined && previous !== path) return false;
      prefixes.set(path.toLowerCase(), path);
    }
  }
  return true;
}

export function sortedFiles(files: RustSourceCoverage[]): boolean {
  const paths = files.map((file) => encoder.encode(file.path));
  return paths.every((path, index) => {
    if (index === 0) return true;
    const previous = paths[index - 1];
    for (
      let offset = 0;
      offset < Math.min(path.length, previous.length);
      offset += 1
    ) {
      if (path[offset] !== previous[offset])
        return path[offset] > previous[offset];
    }
    return path.length > previous.length;
  });
}

export interface RustEvidenceId {
  file: number;
  start: number;
  end: number;
  root: boolean;
}
export function evidenceId(
  value: unknown,
  kind: 'context' | 'call',
  files: RustSourceCoverage[],
): RustEvidenceId | null {
  if (typeof value !== 'string' || value.length > 64) return null;
  const root = kind === 'context' ? /^f(0|[1-9]\d*):root$/.exec(value) : null;
  const match =
    root ??
    new RegExp(`^f(0|[1-9]\\d*):${kind}:(0|[1-9]\\d*):(0|[1-9]\\d*)$`).exec(
      value,
    );
  if (!match) return null;
  const file = Number(match[1]);
  if (!integer(file, files.length - 1)) return null;
  const start = root ? 0 : Number(match[2]);
  const end = root ? files[file].bytes : Number(match[3]);
  if (
    !integer(start, files[file].bytes) ||
    !integer(end, files[file].bytes) ||
    (!root && start >= end)
  )
    return null;
  return { file, start, end, root: root !== null };
}

export function location(
  value: unknown,
  evidence: RustEvidenceId,
  files: RustSourceCoverage[],
): value is RustSourceLocation {
  return (
    record(value) &&
    value.path === files[evidence.file].path &&
    integer(value.line, evidence.start + 1, 1) &&
    integer(value.column, evidence.start + 1, 1) &&
    Number(value.line) + Number(value.column) <= evidence.start + 2 &&
    (value.line !== 1 || value.column === evidence.start + 1) &&
    (!evidence.root || (value.line === 1 && value.column === 1))
  );
}

export function containsEvidence(
  parent: string,
  child: string,
  kind: 'context' | 'call',
  files: RustSourceCoverage[],
): boolean {
  const outer = evidenceId(parent, 'context', files);
  const inner = evidenceId(child, kind, files);
  return (
    outer !== null &&
    inner !== null &&
    outer.file === inner.file &&
    outer.start <= inner.start &&
    outer.end >= inner.end
  );
}
