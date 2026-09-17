import type { DraftRecord } from './draftStore';
export function exportDrafts(records: DraftRecord[]) {
  const url = URL.createObjectURL(
    new Blob([JSON.stringify({ version: 1, drafts: records }, null, 2)], {
      type: 'application/json;charset=utf-8',
    }),
  );
  const link = document.createElement('a');
  link.href = url;
  link.download = 'Studiolo-内存草稿.json';
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}
