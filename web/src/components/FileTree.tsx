import { useId, useState } from 'react';
import { motion } from 'motion/react';
import { useReducedMotionPreference } from '../useReducedMotionPreference';
import type { OpenSource } from '../types';
import { TREE_PAGE_SIZE, type TreeEntry } from '../treeTypes';
import { useDirectory } from '../useDirectory';
import { Button, Cell, CellHeader, ErrorState, GridFrame } from '../ui';
import { AttachmentInfo } from './AttachmentInfo';
import { Icon } from './Icon';
import { FileTypeIcon } from './FileTypeIcon';
import '../styles/tree.css';

interface TreeActions {
  generation: number;
  onOpen: OpenSource;
  onRefresh: () => void;
  onAttachment: (entry: TreeEntry) => void;
  reduceMotion: boolean;
}

/**
 * 文件树（只读副本）：目录按需读取、每页至多 200 项；缩进是 1px 竖线，类型图标单色。
 * 附件只看元数据（AttachmentInfo 浮层），笔记打开阅读器第 1 行。
 */
export function FileTree({
  generation,
  onOpen,
  onRefresh,
}: Omit<TreeActions, 'onAttachment' | 'reduceMotion'>) {
  const [attachment, setAttachment] = useState<TreeEntry | null>(null);
  const reduceMotion = useReducedMotionPreference();
  return (
    <section className="file-tree" aria-label="文件树">
      <GridFrame className="file-tree-sheet">
        <Cell as="div" className="file-tree-intro">
          <CellHeader title="只读副本" meta="目录 · 笔记 · 附件" />
          <p className="file-tree-note">
            展开目录按需读取，每页最多 200 项。附件仅展示路径、后缀与大小。
          </p>
        </Cell>
        <Cell as="div" flush className="file-tree-body">
          <Directory
            path=""
            generation={generation}
            onOpen={onOpen}
            onRefresh={onRefresh}
            onAttachment={setAttachment}
            reduceMotion={reduceMotion}
          />
        </Cell>
      </GridFrame>
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
          <div className="tree-feedback">
            <ErrorState
              title={
                directory.stale
                  ? '快照已更新，请刷新工作台。'
                  : '无法读取目录。'
              }
              onRetry={directory.stale ? actions.onRefresh : directory.retry}
              retryLabel={directory.stale ? '刷新工作台' : '重试目录'}
            >
              <p>{directory.error}</p>
            </ErrorState>
          </div>
        ) : directory.loading ? (
          <p className="tree-feedback tree-feedback--quiet" role="status">
            正在读取目录…
          </p>
        ) : page?.total === 0 ? (
          <p className="tree-feedback tree-feedback--quiet">
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
          <Button
            variant="text"
            size="sm"
            aria-disabled={offset === 0 || directory.loading}
            onClick={() => {
              if (offset > 0 && !directory.loading)
                setOffset((value) => Math.max(0, value - TREE_PAGE_SIZE));
            }}
          >
            上一页
          </Button>
          <span className="tree-pagination__range" aria-live="polite">
            {page
              ? `${page.total === 0 ? 0 : offset + 1}–${offset + page.entries.length} / ${page.total} 项`
              : `第 ${offset / TREE_PAGE_SIZE + 1} 页`}
          </span>
          <Button
            variant="text"
            size="sm"
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
          </Button>
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
        type="button"
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
        // 展开只做状态变化的轻反馈：120ms 透明度＋3px 位移，不用弹簧；减少动态时只留透明度。
        <motion.div
          className="tree-children"
          id={childrenId}
          initial={{ opacity: 0, y: actions.reduceMotion ? 0 : -3 }}
          animate={{ opacity: 1, y: 0 }}
          transformTemplate={actions.reduceMotion ? () => 'none' : undefined}
          transition={{ duration: 0.12, ease: 'easeOut' }}
        >
          <Directory path={entry.path} {...actions} />
        </motion.div>
      ) : null}
    </li>
  );
}
