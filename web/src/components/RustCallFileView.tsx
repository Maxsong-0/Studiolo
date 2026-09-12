import { useState } from 'react';
import { rustCallKinds, rustContextKinds } from '../rustCallSites';
import type {
  RustCallGraph,
  RustCallKind,
  RustSourceCoverage,
} from '../rustCallSites';
import { RustFileCoverage } from './RustFileCoverage';
import { RustSourcePosition } from './RustSourcePosition';

export function RustCallFileView({
  graph,
  file,
}: {
  graph: RustCallGraph;
  file: RustSourceCoverage;
}) {
  const [contextId, setContextId] = useState('all');
  const [kind, setKind] = useState<'all' | RustCallKind>('all');
  const contexts = graph.contexts.filter(
    (item) => item.source.path === file.path,
  );
  const contextMap = new Map(contexts.map((item) => [item.id, item]));
  const fileCalls = graph.calls.filter(
    (item) => item.source.path === file.path,
  );
  const calls = fileCalls.filter(
    (item) =>
      (contextId === 'all' || item.context === contextId) &&
      (kind === 'all' || item.kind === kind),
  );
  return (
    <div className="rust-file-view">
      <RustFileCoverage file={file} />
      <p className="operation-muted">
        {contexts.length} 个词法上下文 · {fileCalls.length} 处调用语法位置。
        包含关系不表示执行顺序；选择上下文只显示直接归属它的位置。
      </p>
      <div className="rust-contexts" role="group" aria-label="Rust 词法上下文">
        <button
          aria-pressed={contextId === 'all'}
          onClick={() => setContextId('all')}
        >
          全部上下文
        </button>
        {contexts.map((context) => {
          const parent = context.parent ? contextMap.get(context.parent) : null;
          return (
            <button
              key={context.id}
              aria-pressed={contextId === context.id}
              onClick={() => setContextId(context.id)}
            >
              <strong>
                {rustContextKinds[context.kind]}
                {context.name ? ` · ${context.name}` : ''}
              </strong>
              <span>
                第 {context.source.line} 行 · 字节列 {context.source.column}
              </span>
              <span>
                所属：
                {parent
                  ? (parent.name ?? rustContextKinds[parent.kind])
                  : '无（文件根）'}
              </span>
            </button>
          );
        })}
      </div>
      <div className="rust-call-filter">
        <label htmlFor="rust-call-kind">调用语法种类</label>
        <select
          id="rust-call-kind"
          value={kind}
          onChange={(event) => setKind(event.target.value as typeof kind)}
        >
          <option value="all">全部种类</option>
          {Object.entries(rustCallKinds).map(([value, label]) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
        <span className="operation-muted">当前 {calls.length} 处</span>
      </div>
      {calls.length ? (
        <ol className="rust-call-list" aria-label="Rust 调用位置">
          {calls.map((call) => {
            const context = contextMap.get(call.context)!;
            return (
              <li key={call.id}>
                <strong>{call.name ?? '表达式目标未提取'}</strong>
                <p>{rustCallKinds[call.kind]} · 目标未解析</p>
                <p>
                  最近词法上下文：
                  {context.name ?? rustContextKinds[context.kind]}
                </p>
                <RustSourcePosition source={call.source} />
              </li>
            );
          })}
        </ol>
      ) : (
        <p className="operation-unavailable" role="status">
          {fileCalls.length === 0
            ? '此白名单文件没有提取到调用位置；不表示文件无行为，也不覆盖其他文件。'
            : '当前上下文与种类没有匹配位置；其他上下文或种类仍可能有结果。'}
        </p>
      )}
    </div>
  );
}
