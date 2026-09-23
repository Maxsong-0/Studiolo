import { Component } from 'react';
import type { ReactNode } from 'react';
import type { OperationDirectory } from '../projectOperations';
import type { OpenSource } from '../types';
import { Button, Cell } from '../ui';
import '../styles/operations.css';

interface DiagramBoundaryProps {
  directories: OperationDirectory[];
  onOpen: OpenSource;
  children: ReactNode;
}

/**
 * 目录图意外渲染失败时，整行换成有来源的纯文本（原因、每个目录的职责与原文入口）。
 * 回退里不放任何图形，职责与原因都按纯文本显示。
 */
export class OperationDiagramBoundary extends Component<
  DiagramBoundaryProps,
  { error: string | null }
> {
  state = { error: null as string | null };

  static getDerivedStateFromError(error: unknown) {
    return {
      error: error instanceof Error ? error.message : '未知的图形渲染错误',
    };
  }

  render() {
    if (this.state.error === null) return this.props.children;
    return (
      <Cell
        span={12}
        as="section"
        className="operation-fallback"
        aria-label="目录与模块职责文本数据"
      >
        <h3>图形暂时无法显示，已切换为文本</h3>
        <p role="alert">原因：{this.state.error}</p>
        <ul>
          {this.props.directories.map((directory) => (
            <li key={directory.path}>
              <strong>{directory.path}</strong>
              <p>
                {directory.source
                  ? directory.label
                  : '路径推导的父目录 · 未声明职责'}
              </p>
              {directory.source ? (
                <>
                  <p className="operation-source-location">
                    {directory.source.path} · 第 {directory.source.line} 行
                  </p>
                  <Button
                    variant="text"
                    size="sm"
                    aria-label={`${directory.path}，从文本查看职责声明来源`}
                    onClick={() => this.props.onOpen(directory.source!)}
                  >
                    查看职责声明原文
                  </Button>
                </>
              ) : (
                <p className="operation-muted">没有可定位的职责声明来源。</p>
              )}
            </li>
          ))}
        </ul>
      </Cell>
    );
  }
}
