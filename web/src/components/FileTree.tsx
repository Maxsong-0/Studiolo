import { useId, useState } from 'react';
import { motion } from 'motion/react';
import { useReducedMotionPreference } from '../useReducedMotionPreference';
import type { OpenSource } from '../types';
import { TREE_PAGE_SIZE, type TreeEntry } from '../treeTypes';
import { useDirectory } from '../useDirectory';
import { AttachmentInfo } from './AttachmentInfo';
import { Icon } from './Icon';
import { FileTypeIcon } from './FileTypeIcon';

interface TreeActions {
  generation: number;
  onOpen: OpenSource;
  onRefresh: () => void;
  onAttachment: (entry: TreeEntry) => void;
  reduceMotion: boolean;
}

export function FileTree({
  generation,
  onOpen,
  onRefresh,
}: Omit<TreeActions, 'onAttachment' | 'reduceMotion'>) {
  const [attachment, setAttachment] = useState<TreeEntry | null>(null);
  const reduceMotion = useReducedMotionPreference();
  return (
    <section className="file-tree" aria-label="文件树">
      <div className="file-tree-heading">
        <h2>只读副本</h2>
        <span>目录 · 笔记 · 附件</span>
      </div>
      <p className="file-tree-note">
        展开目录按需读取，每页最多 200 项。附件仅展示路径、后缀与大小。
      </p>
      <Directory
        path=""
        generation={generation}
        onOpen={onOpen}
        onRefresh={onRefresh}
        onAttachment={setAttachment}
        reduceMotion={reduceMotion}
      />
      {attachment ? (
        <AttachmentInfo
          entry={attachment}
          onClose={() => setAttachment(null)}
        />
      ) : null}
    </section>
  );
}

function Directory({ path, ...actions }: TreeActions & { path: string }) {
  const [offset, setOffset] = useState(0);
  const [hasPaged, setHasPaged] = useState(false);
  const directory = useDirectory(path, offset, actions.generation);
  const page = directory.result;
  const label = path || '只读副本根目录';
  const showPagination = hasPaged || (page?.total ?? 0) > TREE_PAGE_SIZE;
  return (
    <div className="tree-directory">
      <div aria-live="polite">
        {directory.error ? (
          <div className="tree-feedback" role="alert">
            <p>
              {directory.stale
                ? '快照已更新，请刷新工作台。'
                : '无法读取目录。'}
            </p>
            <p>{directory.error}</p>
            <button
              className="text-button"
              onClick={directory.stale ? actions.onRefresh : directory.retry}
            >
              {directory.stale ? '刷新工作台' : '重试目录'}
            </button>
          </div>
        ) : directory.loading ? (
          <p className="tree-feedback" role="status">
            正在读取目录…
          </p>
        ) : page?.total === 0 ? (
          <p className="tree-feedback">
            {path ? '此目录为空。' : '当前快照没有可显示的文件或目录。'}
          </p>
        ) : null}
      </div>
      {page ? (
        <ul className="tree-entries" aria-label={label} key={offset}>
          {page.entries.map((entry) => (
            <Entry key={entry.path} entry={entry} {...actions} />
          ))}
        </ul>
      ) : null}
      {/* 翻页控件不随页面卸载，键盘焦点仍停留在触发按钮。 */}
      {showPagination ? (
        <div className="tree-pagination" aria-label={`${label}分页`}>
          <button
            className="text-button"
            aria-disabled={offset === 0 || directory.loading}
            onClick={() => {
              if (offset > 0 && !directory.loading)
                setOffset((value) => Math.max(0, value - TREE_PAGE_SIZE));
            }}
          >
            上一页
          </button>
          <span aria-live="polite">
            {page
              ? `${page.total === 0 ? 0 : offset + 1}–${offset + page.entries.length} / ${page.total} 项`
              : `第 ${offset / TREE_PAGE_SIZE + 1} 页`}
          </span>
          <button
            className="text-button"
            aria-disabled={!page || page.next_offset === null}
            onClick={() => {
              if (
                page?.next_offset !== null &&
                page?.next_offset !== undefined
              ) {
                setHasPaged(true);
                setOffset(page.next_offset);
              }
            }}
          >
            下一页
          </button>
        </div>
      ) : null}
    </div>
  );
}

function Entry({ entry, ...actions }: TreeActions & { entry: TreeEntry }) {
  const [expanded, setExpanded] = useState(false);
  const childrenId = useId();
  const directory = entry.kind === 'directory';
  const label = directory
    ? `${expanded ? '折叠' : '展开'}目录 ${entry.name}`
    : `${entry.name}，${entry.kind === 'markdown' ? '查看原文' : '查看附件信息'}`;
  return (
    <li className="tree-entry">
      <button
        className="tree-entry-button"
        aria-label={label}
        aria-expanded={directory ? expanded : undefined}
        aria-controls={directory && expanded ? childrenId : undefined}
        onClick={() => {
          if (directory) setExpanded((value) => !value);
          else if (entry.kind === 'markdown')
            actions.onOpen({ path: entry.path, line: 1 });
          else actions.onAttachment(entry);
        }}
      >
        {directory ? (
          <Icon
            name="chevron"
            className={`tree-chevron${expanded ? ' expanded' : ''}`}
          />
        ) : (
          <span className="tree-chevron-placeholder" />
        )}
        <FileTypeIcon entry={entry} />
        <span className="tree-entry-name">{entry.name}</span>
        <span className="tree-entry-meta">
          {directory
            ? `${entry.child_count} 项`
            : entry.kind === 'markdown'
              ? '笔记'
              : '附件'}
        </span>
      </button>
      {directory && expanded ? (
        <motion.div
          className="tree-children"
          id={childrenId}
          initial={{ opacity: 0, y: actions.reduceMotion ? 0 : -3 }}
          animate={{ opacity: 1, y: 0 }}
          transformTemplate={actions.reduceMotion ? () => 'none' : undefined}
          transition={
            actions.reduceMotion
              ? { duration: 0.12 }
              : { type: 'spring', stiffness: 450, damping: 42 }
          }
        >
          <Directory path={entry.path} {...actions} />
        </motion.div>
      ) : null}
    </li>
  );
}
