/**
 * 文档面：`/api/tree`（目录树分页）、`/api/search`（全文与路径）、`/api/document`（原文与导航）。
 *
 * 全部从假 Vault 现算：大纲来自标题，出链来自 `[[wiki 链接]]`（唯一命中＝resolved，多篇同名＝
 * ambiguous，找不到＝missing，标题不存在＝missing_heading），反链是别的笔记指向本篇的出链。
 * 形状以 `treeQuery.ts`、`searchQuery.ts`、`validate.ts`（validateDocument）为准。
 */
import { noteTitle } from './vault.ts';
import type { VaultAttachment, VaultNote } from './vault.ts';

export const TREE_PAGE_SIZE = 200;
const bytesOf = (text: string) => new TextEncoder().encode(text).length;

type Entry = {
  path: string;
  name: string;
  kind: 'directory' | 'markdown' | 'attachment';
  size_bytes: string | null;
  child_count: number | null;
};

/** 某目录下的直接子项：目录在前，文件在后，各自按名称排序。null 表示不是目录。 */
export function listDirectory(
  notes: VaultNote[],
  attachments: VaultAttachment[],
  directory: string,
): Entry[] | null {
  const files = [
    ...notes.map((note) => ({
      path: note.path,
      bytes: bytesOf(note.content),
      markdown: true,
    })),
    ...attachments.map((item) => ({
      path: item.path,
      bytes: item.bytes,
      markdown: false,
    })),
  ];
  const prefix = directory ? `${directory}/` : '';
  if (directory && !files.some((file) => file.path.startsWith(prefix)))
    return null;
  const directories = new Map<string, Set<string>>();
  const entries: Entry[] = [];
  for (const file of files) {
    if (!file.path.startsWith(prefix)) continue;
    const rest = file.path.slice(prefix.length).split('/');
    if (rest.length === 1)
      entries.push({
        path: file.path,
        name: rest[0],
        kind: file.markdown ? 'markdown' : 'attachment',
        size_bytes: String(file.bytes),
        child_count: null,
      });
    else {
      const children = directories.get(rest[0]) ?? new Set<string>();
      children.add(rest[1]);
      directories.set(rest[0], children);
    }
  }
  const byName = (left: Entry, right: Entry) =>
    left.name < right.name ? -1 : left.name > right.name ? 1 : 0;
  const folders = [...directories].map(([name, children]): Entry => ({
    path: `${prefix}${name}`,
    name,
    kind: 'directory',
    size_bytes: null,
    child_count: children.size,
  }));
  return [...folders.sort(byName), ...entries.sort(byName)];
}

export function treePage(
  notes: VaultNote[],
  attachments: VaultAttachment[],
  path: string,
  offset: number,
  generation: number,
) {
  const entries = listDirectory(notes, attachments, path);
  if (!entries) return null;
  const end = Math.min(entries.length, offset + TREE_PAGE_SIZE);
  return {
    generation,
    path,
    offset,
    total: entries.length,
    entries: entries.slice(offset, end),
    next_offset: end < entries.length ? end : null,
  };
}

export function search(
  notes: VaultNote[],
  query: string,
  mode: 'full_text' | 'path',
  project: string | null,
  generation: number,
) {
  const needle = query.toLocaleLowerCase();
  const scoped = notes.filter(
    (note) => !project || note.path.startsWith(`${project}/`),
  );
  const hits: {
    path: string;
    title: string;
    line: number;
    snippet: string;
    kind: 'path' | 'content';
  }[] = [];
  for (const note of scoped) {
    const title = noteTitle(note);
    if (mode === 'path') {
      if (`${note.path} ${title}`.toLocaleLowerCase().includes(needle))
        hits.push({
          path: note.path,
          title,
          line: 1,
          snippet: note.path,
          kind: 'path',
        });
      continue;
    }
    note.content.split('\n').forEach((line, index) => {
      if (line.toLocaleLowerCase().includes(needle))
        hits.push({
          path: note.path,
          title,
          line: index + 1,
          snippet: line.trim().slice(0, 160),
          kind: 'content',
        });
    });
  }
  return { generation, hits: hits.slice(0, 100), truncated: hits.length > 100 };
}

/** 代码围栏之外的行（1 起算）。 */
function visibleLines(note: VaultNote) {
  const lines: { text: string; line: number }[] = [];
  let fenced = false;
  note.content.split('\n').forEach((text, index) => {
    if (text.startsWith('```')) fenced = !fenced;
    else if (!fenced) lines.push({ text, line: index + 1 });
  });
  return lines;
}

function outline(note: VaultNote) {
  return visibleLines(note).flatMap(({ text, line }) => {
    const heading = /^(#{1,6}) (.+)$/.exec(text);
    return heading
      ? [
          {
            title: heading[2].trim(),
            level: heading[1].length,
            source: { path: note.path, line },
          },
        ]
      : [];
  });
}

const stem = (path: string) => path.split('/').at(-1)!.replace(/\.md$/i, '');

function resolve(notes: VaultNote[], target: string) {
  const [name, heading] = target.split('#');
  const candidates = notes.filter((note) =>
    name.includes('/')
      ? note.path === name || note.path === `${name}.md`
      : stem(note.path) === name,
  );
  if (candidates.length !== 1)
    return {
      status: candidates.length ? 'ambiguous' : 'missing',
      targets: candidates.map((note) => ({
        path: note.path,
        line: 1,
        title: noteTitle(note),
      })),
    };
  const [note] = candidates;
  if (!heading)
    return {
      status: 'resolved',
      targets: [{ path: note.path, line: 1, title: noteTitle(note) }],
    };
  const found = outline(note).find((item) => item.title === heading);
  return found
    ? {
        status: 'resolved',
        targets: [
          { path: note.path, line: found.source.line, title: found.title },
        ],
      }
    : {
        status: 'missing_heading',
        targets: [{ path: note.path, line: 1, title: noteTitle(note) }],
      };
}

function outgoing(notes: VaultNote[], note: VaultNote) {
  return visibleLines(note).flatMap(({ text, line }) =>
    [...text.matchAll(/\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g)].map((match) => ({
      label: (match[2] ?? match[1]).trim(),
      raw: match[0],
      source: { path: note.path, line },
      ...resolve(notes, match[1].trim()),
    })),
  );
}

export function documentView(notes: VaultNote[], path: string) {
  const note = notes.find((item) => item.path === path);
  if (!note) return null;
  const backlinks = notes
    .filter((other) => other.path !== path)
    .flatMap((other) =>
      outgoing(notes, other)
        .filter(
          (link) => link.status === 'resolved' && link.targets[0].path === path,
        )
        .map((link) => ({
          title: noteTitle(other),
          source: link.source,
          context: other.content.split('\n')[link.source.line - 1].trim(),
        })),
    );
  return {
    path,
    title: noteTitle(note),
    content: note.content,
    navigation: {
      outline: outline(note),
      outgoing: outgoing(notes, note),
      backlinks,
      truncated: false,
    },
  };
}
