import { useMemo, useState } from 'react';
import type { OperationDirectory } from '../projectOperations';
import type { OpenSource } from '../types';
import { Icon } from './Icon';

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
            className="operation-node"
            aria-pressed={selected === directory.path}
            aria-label={`选择目录 ${directory.path}`}
            onClick={() => onSelect(directory.path)}
          >
            <Icon name="folder" />
            <span>
              <strong>{directory.path}</strong>
              <span className="operation-node-label">
                {directory.source
                  ? directory.label
                  : '路径推导的父目录 · 未声明职责'}
              </span>
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
    <div className="operation-diagram-layout">
      <div
        className="operation-diagram-scroll"
        role="region"
        aria-label="目录与模块职责图"
        tabIndex={0}
      >
        <div className="operation-diagram">
          <p className="operation-graph-root">
            <Icon name="folder" />
            仓库目录
          </p>
          <DirectoryBranches
            nodes={tree}
            selected={selected.path}
            onSelect={setSelection}
          />
        </div>
      </div>
      <aside
        className="operation-selection"
        aria-label="所选目录详情"
        aria-live="polite"
      >
        <span className="operation-eyebrow">所选目录</span>
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
            <button
              className="text-button"
              onClick={() => onOpen(selected.source!)}
            >
              查看职责声明原文
              <Icon name="arrow" />
            </button>
          </>
        ) : (
          <p className="operation-muted">没有可定位的职责声明来源。</p>
        )}
      </aside>
    </div>
  );
}
