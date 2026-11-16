/**
 * 逐行归属（T-210）：当前正文每一行最后是哪一版写的。封闭解码；服务端说不清的照实显示，网页不补猜。
 */
import { historyActor, type HistoryActor } from './historyModel';

export type BlameOrigin = 'version' | 'untracked' | 'beyond_history';
export interface BlameRun {
  from_line: number;
  to_line: number;
  origin: BlameOrigin;
  history_id: string | null;
  revision: string | null;
  author: HistoryActor | null;
  authored_at_ms: number | null;
}
export interface Blame {
  vault_uuid: string;
  path: string;
  revision: string;
  content_sha256: string;
  lines: number;
  versions: number;
  more_history: boolean;
  runs: BlameRun[];
}

const RUN_KEYS = [
  'from_line',
  'to_line',
  'origin',
  'history_id',
  'revision',
  'author',
  'authored_at_ms',
];
const KEYS = [
  'vault_uuid',
  'path',
  'revision',
  'content_sha256',
  'lines',
  'versions',
  'more_history',
  'runs',
];
const record = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);
const exact = (value: Record<string, unknown>, keys: string[]) =>
  Object.keys(value).length === keys.length &&
  keys.every((key) => Object.hasOwn(value, key));
const hex = (value: unknown): value is string =>
  typeof value === 'string' && /^[0-9a-f]{64}$/.test(value);
const count = (value: unknown, max: number): value is number =>
  typeof value === 'number' &&
  Number.isSafeInteger(value) &&
  value >= 0 &&
  value <= max;
function invalid(): never {
  throw new Error('逐行归属响应格式无效，请重新读取。');
}

/** 各段必须从第 1 行起首尾相接、正好盖满全部行；不是「某一版」的段不许带版本信息。 */
export function blame(value: unknown, path: string): Blame {
  if (
    !record(value) ||
    !exact(value, KEYS) ||
    typeof value.vault_uuid !== 'string' ||
    value.path !== path ||
    !hex(value.revision) ||
    !hex(value.content_sha256) ||
    !count(value.lines, 20_000) ||
    !count(value.versions, 50) ||
    typeof value.more_history !== 'boolean' ||
    !Array.isArray(value.runs)
  )
    invalid();
  let next = 1;
  for (const run of value.runs as unknown[]) {
    if (
      !record(run) ||
      !exact(run, RUN_KEYS) ||
      run.from_line !== next ||
      !count(run.to_line, value.lines) ||
      run.to_line < next ||
      !historyActor(run.author) ||
      (run.authored_at_ms !== null && !count(run.authored_at_ms, 8.64e15))
    )
      invalid();
    const versioned = run.origin === 'version';
    if (
      (!versioned &&
        run.origin !== 'untracked' &&
        run.origin !== 'beyond_history') ||
      (versioned
        ? !hex(run.history_id) || !hex(run.revision)
        : run.history_id !== null ||
          run.revision !== null ||
          run.author !== null ||
          run.authored_at_ms !== null)
    )
      invalid();
    next = run.to_line + 1;
  }
  if (next !== value.lines + 1) invalid();
  return value as unknown as Blame;
}

export type BlameKind =
  'human' | 'obsidian' | 'agent' | 'unknown' | 'untracked' | 'beyond';
export interface BlameMark {
  kind: BlameKind;
  /** 窄带上写的缩写：不只靠线型区分。 */
  short: string;
  /** 悬停、聚焦和读屏听到的整句。 */
  title: string;
}

function when(ms: number | null) {
  if (ms === null) return '时间未知';
  const date = new Date(ms);
  return Number.isNaN(date.getTime()) ? '时间未知' : date.toLocaleString();
}

export function blameMark(run: BlameRun): BlameMark {
  const range =
    run.from_line === run.to_line
      ? `第 ${run.from_line} 行`
      : `第 ${run.from_line}–${run.to_line} 行`;
  if (run.origin === 'untracked')
    return {
      kind: 'untracked',
      short: '外',
      title: `${range}：库外直接改的，没有记录是谁`,
    };
  if (run.origin === 'beyond_history')
    return {
      kind: 'beyond',
      short: '早',
      title: `${range}：早于可查历史，更早的版本没有记录或已清理`,
    };
  const version = `版本 ${run.revision?.slice(0, 8)} · ${when(run.authored_at_ms)}`;
  const author = run.author;
  if (author === null)
    return {
      kind: 'unknown',
      short: '?',
      title: `${range}：这一版的作者没有记录 · ${version}`,
    };
  if (author.kind === 'agent')
    return {
      kind: 'agent',
      short: 'AI',
      title: `${range}：AI ${author.agent_type}（${author.instance_id}）${author.task_id ? ` · 任务 ${author.task_id}` : ''} · ${version}`,
    };
  if (author.kind === 'obsidian')
    return {
      kind: 'obsidian',
      short: 'Ob',
      title: `${range}：Obsidian 同步 · ${version}`,
    };
  return {
    kind: 'human',
    short: '人',
    title: `${range}：人 ${author.subject} · ${version}`,
  };
}

/** 正文的普通 SHA-256（十六进制），和服务端的 `content_sha256` 比，确认两边说的是同一版。 */
export async function sha256Hex(text: string) {
  const digest = await crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(text),
  );
  return [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, '0'))
    .join('');
}
