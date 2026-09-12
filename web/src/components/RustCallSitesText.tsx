import { rustCallKinds, rustContextKinds } from '../rustCallSites';
import type { RustCallGraph } from '../rustCallSites';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { RustFileCoverage } from './RustFileCoverage';
import { RustFileDeclarations } from './RustFileDeclarations';
import { RustSourcePosition } from './RustSourcePosition';
import { RustCallAssociationText } from './RustCallAssociationText';
import { RustCallReferencesText } from './RustCallReferencesText';

export function RustCallSitesText({
  graph,
  data,
  onOpen,
}: {
  graph: RustCallGraph;
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const links = data.rust_call_sites?.links;
  const associations = new Map(
    links?.associations.map((item) => [item.call_id, item]),
  );
  const contexts = new Map(graph.contexts.map((item) => [item.id, item]));
  return (
    <div
      className="rust-text-data"
      role="region"
      aria-label="Rust 调用位置文本数据"
    >
      {graph.files.map((file) => (
        <section key={file.path}>
          <h3>{file.path}</h3>
          <RustFileCoverage file={file} />
          <h4>词法上下文（包含关系不表示执行顺序）</h4>
          <ul>
            {graph.contexts
              .filter((item) => item.source.path === file.path)
              .map((context) => (
                <li key={context.id}>
                  {rustContextKinds[context.kind]} · {context.name ?? '匿名'}
                  <p>
                    ID：{context.id}；所属：{context.parent ?? '无（文件根）'}
                  </p>
                  <RustSourcePosition source={context.source} />
                  {links && context.kind === 'function' ? (
                    <RustCallReferencesText
                      target={context.id}
                      graph={graph}
                      links={links}
                    />
                  ) : null}
                </li>
              ))}
          </ul>
          <h4>调用语法位置（词法关联不表示编译器名字解析）</h4>
          <ul>
            {graph.calls
              .filter((item) => item.source.path === file.path)
              .map((call) => {
                const association = associations.get(call.id);
                return (
                  <li key={call.id}>
                    {rustCallKinds[call.kind]} ·{' '}
                    {call.name ?? '表达式目标未提取'}
                    <p>最近词法上下文：{call.context}</p>
                    <RustSourcePosition source={call.source} />
                    <RustCallAssociationText
                      association={association}
                      target={
                        association?.target
                          ? contexts.get(association.target)
                          : undefined
                      }
                    />
                  </li>
                );
              })}
          </ul>
          {!graph.calls.some((item) => item.source.path === file.path) ? (
            <p>
              此白名单文件没有提取到调用位置；不表示文件无行为，也不覆盖其他文件。
            </p>
          ) : null}
          <RustFileDeclarations path={file.path} data={data} onOpen={onOpen} />
        </section>
      ))}
    </div>
  );
}
