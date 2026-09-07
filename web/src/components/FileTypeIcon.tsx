import type { TreeEntry } from '../treeTypes';
import { Icon } from './Icon';

type FileIconName =
  'folder' | 'book' | 'image' | 'table' | 'canvas' | 'code' | 'attachment';

const imageExtensions = new Set([
  'png',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'avif',
  'heic',
  'heif',
  'tif',
  'tiff',
  'bmp',
  'ico',
  'svg',
]);
const tableExtensions = new Set(['csv', 'tsv', 'xls', 'xlsx', 'ods']);
const canvasExtensions = new Set(['canvas', 'excalidraw']);
const codeExtensions = new Set([
  'html',
  'htm',
  'js',
  'mjs',
  'cjs',
  'jsx',
  'ts',
  'tsx',
  'json',
  'css',
  'scss',
  'sass',
  'less',
  'xml',
  'yaml',
  'yml',
  'toml',
  'rs',
  'py',
  'sh',
  'bash',
  'zsh',
  'sql',
  'go',
  'java',
  'c',
  'cpp',
  'h',
  'hpp',
  'rb',
  'swift',
  'kt',
  'vue',
  'svelte',
]);

function iconFor(entry: Pick<TreeEntry, 'kind' | 'name'>): FileIconName {
  if (entry.kind === 'directory') return 'folder';
  if (entry.kind === 'markdown') return 'book';
  // 仅根据最后一个明示后缀提示类型，不读取内容、不鉴定 MIME。
  const dot = entry.name.lastIndexOf('.');
  const extension = dot > 0 ? entry.name.slice(dot + 1).toLowerCase() : '';
  if (imageExtensions.has(extension)) return 'image';
  if (tableExtensions.has(extension)) return 'table';
  if (canvasExtensions.has(extension)) return 'canvas';
  if (codeExtensions.has(extension)) return 'code';
  return 'attachment';
}

export function FileTypeIcon({
  entry,
  badge = false,
}: {
  entry: Pick<TreeEntry, 'kind' | 'name'>;
  badge?: boolean;
}) {
  const icon = iconFor(entry);
  return (
    <span
      className={`file-type-icon file-type-${icon}${badge ? ' file-type-badge' : ''}`}
      aria-hidden="true"
    >
      <Icon name={icon} />
    </span>
  );
}
