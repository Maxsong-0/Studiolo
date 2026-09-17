import type { HistoryActor, HistoryEntry } from '../historyModel';

function actor(value: HistoryActor | null) {
  if (value === null) return '未知';
  return value.kind === 'obsidian'
    ? 'Obsidian'
    : `人侧会话 · ${value.issuer} · ${value.subject}`;
}
function time(value: number | null) {
  if (value === null) return '未知';
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? '超出浏览器可显示的时间范围'
    : date.toLocaleString();
}
/** 提交与归档是两件事，旧作者未知时不从归档者或时间补推。 */
export function HistoryMetadata({ entry }: { entry: HistoryEntry }) {
  return (
    <dl className="history-metadata">
      <dt>提交时间</dt>
      <dd>{time(entry.authored_at_ms)}</dd>
      <dt>提交主体</dt>
      <dd>{actor(entry.author)}</dd>
      <dt>归档时间</dt>
      <dd>{time(entry.archived_at_ms)}</dd>
      <dt>归档主体</dt>
      <dd>{actor(entry.archived_by)}</dd>
      <dt>正文大小</dt>
      <dd>{entry.bytes.toLocaleString()} 字节</dd>
    </dl>
  );
}
