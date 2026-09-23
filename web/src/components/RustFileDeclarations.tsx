import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { Button } from '../ui';
import '../styles/operations.css';
import '../styles/rust-call-sites.css';

/**
 * 文件所在目录的文档声明：规范目录加「/」的路径前缀命中才算包含，列出全部命中，各给 Vault 原文入口。
 * 这只是目录包含关系，不代表符号或调用目标已经对齐。
 */
export function RustFileDeclarations({
  path,
  data,
  onOpen,
}: {
  path: string;
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const nodes =
    data.architecture?.status === 'ready'
      ? data.architecture.nodes.filter(
          (node) =>
            node.directory !== null && path.startsWith(`${node.directory}/`),
        )
      : [];
  return (
    <aside className="rust-declarations" aria-label="文件所在目录的架构声明">
      <h3>所在目录的文档声明</h3>
      <p className="operation-muted">
        仅表示文件位于声明目录下，不代表符号、模块实现或调用目标已经对齐。
      </p>
      {nodes.length ? (
        <ul aria-label="目录包含的全部架构声明">
          {nodes.map((node) => (
            <li key={node.id}>
              <code>{node.directory}</code>
              <Button
                variant="text"
                size="sm"
                onClick={() => onOpen(node.source)}
                aria-label={`${node.label}，查看所在目录的 Vault 声明`}
              >
                {node.label}
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="operation-muted">
          {data.architecture?.status === 'ready'
            ? '没有声明目录包含此文件。'
            : '架构声明不可用，无法提供目录包含关系。'}
        </p>
      )}
    </aside>
  );
}
