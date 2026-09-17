import { Component } from 'react';
import type { ReactNode } from 'react';
import type { OperationDirectory } from '../projectOperations';
import type { OpenSource } from '../types';

interface DiagramBoundaryProps {
  directories: OperationDirectory[];
  onOpen: OpenSource;
  children: ReactNode;
}

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
      <section
        className="operation-unavailable"
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
                  <p>
                    {directory.source.path} · 第 {directory.source.line} 行
                  </p>
                  <button
                    className="text-button"
                    aria-label={`${directory.path}，从文本查看职责声明来源`}
                    onClick={() => this.props.onOpen(directory.source!)}
                  >
                    查看职责声明原文
                  </button>
                </>
              ) : (
                <p className="operation-muted">没有可定位的职责声明来源。</p>
              )}
            </li>
          ))}
        </ul>
      </section>
    );
  }
}
