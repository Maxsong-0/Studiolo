import { getBlob, MAX_ATTACHMENT_BYTES } from './sessionHttp';
import type { TreeEntry } from './treeTypes';

export const TEXT_PREVIEW_BYTES = 1024 * 1024;
const IMAGES = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp']);
const TEXT_EXTENSIONS = new Set([
  'txt',
  'csv',
  'tsv',
  'json',
  'canvas',
  'yaml',
  'yml',
  'toml',
  'log',
]);

export function attachmentSize(entry: TreeEntry): number | null {
  if (
    entry.kind !== 'attachment' ||
    !entry.size_bytes ||
    !/^(0|[1-9]\d*)$/.test(entry.size_bytes)
  )
    return null;
  const size = Number(entry.size_bytes);
  return Number.isSafeInteger(size) && size <= MAX_ATTACHMENT_BYTES
    ? size
    : null;
}

export function automaticPreview(entry: TreeEntry): boolean {
  const size = attachmentSize(entry);
  const extension = entry.name.split('.').pop()?.toLowerCase() ?? '';
  return (
    size !== null &&
    (['png', 'jpg', 'jpeg', 'gif', 'webp'].includes(extension) ||
      (TEXT_EXTENSIONS.has(extension) && size <= TEXT_PREVIEW_BYTES))
  );
}

export interface Attachment {
  blob: Blob;
  kind: 'image' | 'text' | 'pdf' | 'download';
  text?: string;
  label: string;
}

export async function readAttachment(
  entry: TreeEntry,
  generation: number,
  signal: AbortSignal,
): Promise<Attachment> {
  const size = attachmentSize(entry);
  if (size === null)
    throw new Error('附件超出网页读取上限，请在同步副本中打开。');
  const query = new URLSearchParams({
    path: entry.path,
    generation: String(generation),
  });
  const blob = await getBlob(`/api/attachment?${query}`, signal, size);
  // 类型由服务端按文件头字节判定（crates/local-server/src/attachments/media.rs），前端只认响应头，
  // 所以标签写明是服务端核的；判不出的一律 octet-stream，走下面的仅下载。
  if (IMAGES.has(blob.type))
    return {
      blob,
      kind: 'image',
      label: `${blob.type.slice(6).toUpperCase()} 图片（服务端已按字节校验）`,
    };
  if (blob.type === 'application/pdf')
    return { blob, kind: 'pdf', label: 'PDF（服务端已按字节校验）' };
  // 文本只插入文本节点；下载使用惰性类型，不让扩展名或未知响应类型成为可执行文档。
  const safe = blob.slice(0, blob.size, 'application/octet-stream');
  if (blob.type === 'text/plain; charset=utf-8') {
    if (blob.size > TEXT_PREVIEW_BYTES)
      return {
        blob: safe,
        kind: 'download',
        label: '纯文本（超过 1 MiB，仅下载）',
      };
    return {
      blob: safe,
      kind: 'text',
      text: await blob.text(),
      label: 'UTF-8 纯文本（服务端已按字节校验）',
    };
  }
  return { blob: safe, kind: 'download', label: '此类型仅提供下载' };
}
