import { useMemo, useState } from 'react';
import type { OperationDirectory } from '../projectOperations';
import type { OpenSource } from '../types';
import { Button, Cell } from '../ui';
import { Icon } from './Icon';
import '../styles/operations.css';

interface DirectoryNode {
  directory: OperationDirectory;
  children: DirectoryNode[];
}

function directoryTree(directories: OperationDirectory[]): DirectoryNode[] {
  const nodes = new Map(
    directories.map((directory) => [
      directory.path,
      { directory, children: [] } as DirectoryNode,
    ]),
  );
  const roots: DirectoryNode[] = [];
  for (const [path, node] of nodes) {
    let parent = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
    while (parent && !nodes.has(parent))
      parent = parent.includes('/')
        ? parent.slice(0, parent.lastIndexOf('/'))
        : '';
    const ancestor =
      nodes.get(parent) ?? (path !== '.' ? nodes.get('.') : undefined);
    if (ancestor && ancestor !== node) ancestor.children.push(node);
    else roots.push(node);
  }
  return roots;
}

/** 目录树：每个目录是一个黑线框按钮，父子之间是正交连线（竖线接横线，见 operations.css）。 */
function DirectoryBranches({
  nodes,
  selected,
  onSelect,
}: {
  nodes: DirectoryNode[];
  selected: string;
  onSelect: (path: string) => void;
}) {
  return (
    <ul className="operation-branches">
      {nodes.map(({ directory, children }) => (
        <li key={directory.path}>
          <button
            type="button"
            className={
              directory.source ? 'operation-node' : 'operation-node is-derived'
            }
            aria-pressed={selected === directory.path}
            aria-label={`选择目录 ${directory.path}`}
            onClick={() => onSelect(directory.path)}
          >
            <span className="operation-node__path">{directory.path}</span>
            <span className="operation-node__label">
              {directory.source
                ? directory.label
                : '路径推导的父目录 · 未声明职责'}
            </span>
          </button>
          {children.length ? (
            <DirectoryBranches
              nodes={children}
              selected={selected}
              onSelect={onSelect}
            />
          ) : null}
        </li>
      ))}
    </ul>
  );
}

/**
 * 目录与模块职责：左 8 列网格纸上的目录树，右 4 列所选目录（职责与声明原文入口）。
 * 两格都由这里渲染，选中状态只有一份。
 */
export function OperationDiagram({
  directories,
  onOpen,
}: {
  directories: OperationDirectory[];
  onOpen: OpenSource;
}) {
  const [selection, setSelection] = useState('');
  const tree = useMemo(() => directoryTree(directories), [directories]);
  const selected =
    directories.find((item) => item.path === selection) ??
    directories.find((item) => item.source !== null) ??
    directories[0];

  return (
    <>
      <Cell span={8} surface="grid" as="div">
        <div
          className="operation-tree-scroll"
          role="region"
          aria-label="目录与模块职责图"
          tabIndex={0}
        >
          <div className="operation-tree">
            <p className="operation-tree__root">仓库目录</p>
            <DirectoryBranches
              nodes={tree}
              selected={selected.path}
              onSelect={setSelection}
            />
          </div>
        </div>
      </Cell>
      <Cell
        span={4}
        as="aside"
        className="operation-selection"
        aria-label="所选目录详情"
        aria-live="polite"
      >
        <span className="operation-label">所选目录</span>
        <h3>{selected.path}</h3>
        <p>
          {selected.source
            ? selected.label
            : '此节点由目录路径推导，没有人工声明的模块职责。'}
        </p>
        {selected.source ? (
          <>
            <p className="operation-source-location">
              {selected.source.path} · 第 {selected.source.line} 行
            </p>
            <Button
              variant="text"
              size="sm"
              onClick={() => onOpen(selected.source!)}
            >
              查看职责声明原文
              <Icon name="arrow" />
            </Button>
          </>
        ) : (
          <p className="operation-muted">没有可定位的职责声明来源。</p>
        )}
      </Cell>
    </>
  );
}
