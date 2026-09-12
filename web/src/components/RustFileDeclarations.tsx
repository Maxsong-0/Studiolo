import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';

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
              <button
                className="text-button"
                onClick={() => onOpen(node.source)}
                aria-label={`${node.label}，查看所在目录的 Vault 声明`}
              >
                {node.label}
              </button>
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
