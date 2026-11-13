import { COMMAND_TARGETS } from '../commands/focusTarget';
import { useId } from 'react';
import { useChanges } from '../useChanges';
import { useViewLocation } from '../useViewLocation';
import {
  changeActor,
  changeLabels,
  changeProject,
  changeTime,
  type ChangeEntry,
} from '../changesModel';
import type { OpenSource, Project } from '../types';
import { Button } from '../ui';
import '../styles/changes.css';
const gaps = {
  startup: '服务已重新启动，启动前未收录的外部变化无法还原。',
  retention: '较早的外部观察记录已超过保留上限。',
  explanation_incomplete:
    '本次观察未能完整核对提交历史；来源不明的变化保持未知。',
};
function group(row: ChangeEntry) {
  if (row.path === null) return '观察缺口';
  const parts = row.path.split('/');
  return parts[0] === '项目' && parts.length >= 3
    ? parts.slice(0, 2).join('/')
    : '其他文档';
}
export function ChangesPanel({
  projects,
  onOpen,
  active,
}: {
  projects: Project[];
  onOpen: OpenSource;
  active: boolean;
}) {
  const { location, select } = useViewLocation();
  const project = changeProject(location.project) ? location.project : null;
  return (
    <section className="changes-panel" aria-label="内容变更流">
      <ChangesContent
        key={project ?? 'all'}
        projects={projects}
        project={project}
        active={active}
        onProject={(project) => select({ project })}
        onOpen={onOpen}
      />
    </section>
  );
}
function ChangesContent({
  projects,
  project,
  active,
  onProject,
  onOpen,
}: {
  projects: Project[];
  project: string | null;
  active: boolean;
  onProject: (value: string | null) => void;
  onOpen: OpenSource;
}) {
  const id = useId();
  const feed = useChanges(project, active);
  const grouped = new Map<string, ChangeEntry[]>();
  for (const row of feed.page?.entries ?? []) {
    const name = group(row);
    const rows = grouped.get(name) ?? [];
    rows.push(row);
    grouped.set(name, rows);
  }
  const unread =
    feed.page?.entries.filter(
      (row) => row.kind !== 'gap' && row.unread === true,
    ).length ?? 0;
  return (
    <>
      <div className="changes-toolbar">
        <div>
          <h2>内容变更</h2>
          <p>
            按项目核对谁改了什么。外部变化按成功观察记录，同一轮内的多次修改可能合为一条。
          </p>
        </div>
        <label htmlFor={id}>
          项目范围
          <select
            data-command-target={COMMAND_TARGETS.changesProject}
            id={id}
            value={project ?? ''}
            disabled={feed.marking}
            onChange={(e) => onProject(e.target.value || null)}
          >
            <option value="">全库</option>
            {project && !projects.some((p) => p.root === project) ? (
              <option value={project}>{project}</option>
            ) : null}
            {projects
              .filter((p) => changeProject(p.root))
              .map((p) => (
                <option key={p.id} value={p.root}>
                  {p.name}
                </option>
              ))}
          </select>
        </label>
        <Button
          onClick={() => void feed.reload()}
          disabled={feed.loading || feed.marking}
        >
          重新读取
        </Button>
        <Button
          onClick={() => void feed.mark()}
          disabled={!feed.page || feed.loading || feed.marking || feed.expired}
        >
          {feed.marking ? '正在记录…' : '将本次快照全部标为已读'}
        </Button>
      </div>
      <p className="changes-caption">
        全库与项目的已读记录分别保存，并在你的设备间共用。打开列表不会自动标记。
      </p>
      {feed.loading ? <p role="status">正在读取变更…</p> : null}
      {feed.error ? (
        <p role="alert">
          {feed.unavailable ? '当前服务尚未提供内容变更流。' : feed.error}
          {feed.expired ? ' 请重新读取第一页。' : ''}
        </p>
      ) : null}
      {feed.message ? <p role="status">{feed.message}</p> : null}
      {feed.page ? (
        <>
          <p className="changes-caption">
            本页 {feed.page.entries.length} 条记录 · 已确认未读 {unread} 条
            {feed.page.next_cursor ? ' · 后面还有记录' : ''}
          </p>
          {!feed.page.seen_complete ? (
            <p className="changes-notice">
              已读水位暂不完整；不能确认的条目标为「未读状态未知」。
            </p>
          ) : null}
          {feed.page.history_incomplete || feed.page.retired ? (
            <p className="changes-notice">
              历史存在未收录或已清理的记录，这份列表不能代表所有过去的改动。
            </p>
          ) : null}
          {feed.page.entries.length === 0 ? (
            <p>
              {feed.page.next_cursor
                ? '本段没有匹配记录，可以继续向前查看。'
                : '此范围没有更多可显示的记录。'}
            </p>
          ) : null}
          {[...grouped].map(([name, rows]) => (
            <section className="changes-group" key={name} aria-label={name}>
              <h3>{name}</h3>
              <ul>
                {rows.map((row) => (
                  <ChangeRow key={row.id} row={row} onOpen={onOpen} />
                ))}
              </ul>
            </section>
          ))}
          <div className="changes-pagination">
            <Button
              onClick={() => void feed.next()}
              disabled={!feed.page.next_cursor || feed.loading || feed.marking}
            >
              更早的变化
            </Button>
            <span>每页最多 50 条 · 时间仅供参考</span>
          </div>
        </>
      ) : null}
    </>
  );
}
function ChangeRow({ row, onOpen }: { row: ChangeEntry; onOpen: OpenSource }) {
  const { goTo } = useViewLocation();
  if (row.kind === 'gap')
    return (
      <li className="changes-gap">
        <strong>记录缺口</strong>
        <p>{row.gap ? gaps[row.gap] : '未知缺口'}</p>
        <small>观察于代次 {row.generation}</small>
      </li>
    );
  const readable =
    row.path !== null && /\.md$/i.test(row.path) && row.revision !== null;
  return (
    <li className="changes-row">
      <span
        className={`changes-read${row.unread === true ? ' changes-read--new' : ''}`}
        aria-label={
          row.unread === true
            ? '未读'
            : row.unread === false
              ? '已读'
              : '未读状态未知'
        }
      >
        {row.unread === true ? '●' : row.unread === false ? '○' : '?'}
      </span>
      <div>
        <div className="changes-row__heading">
          <strong>{changeLabels[row.kind]}</strong>
          <span>{changeTime(row.at_ms)}</span>
        </div>
        <p className="changes-path">
          {row.path}
          {row.counterpart ? ` ↔ ${row.counterpart}` : ''}
        </p>
        <p className="changes-caption">
          {changeActor(row)}
          {row.source === 'observation'
            ? ` · 观察于代次 ${row.generation}`
            : ''}
        </p>
        <details>
          <summary>版本信息</summary>
          <dl>
            <dt>之前版本</dt>
            <dd>{row.previous_revision ?? '无可用版本'}</dd>
            <dt>之后版本</dt>
            <dd>{row.revision ?? '路径已删除或移走'}</dd>
            <dt>之后大小</dt>
            <dd>{row.bytes === null ? '未知' : `${row.bytes} 字节`}</dd>
          </dl>
        </details>
      </div>
      <Button
        variant="text"
        onClick={() =>
          readable
            ? onOpen({ path: row.path!, line: 1 })
            : goTo({ view: 'documents', panel: 'tree' })
        }
      >
        {readable ? '打开当前文档' : '查看文件树'}
      </Button>
    </li>
  );
}
