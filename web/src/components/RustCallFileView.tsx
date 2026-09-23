import { useRef, useState } from 'react';
import { rustCallKinds, rustContextKinds } from '../rustCallSites';
import type {
  RustCallGraph,
  RustCallKind,
  RustSourceCoverage,
} from '../rustCallSites';
import { RustFileCoverage } from './RustFileCoverage';
import type { RustCallLinks, RustLinkSelection } from '../rustCallLinks';
import { RustCallLinkDetails } from './RustCallLinkDetails';
import { RustCallList } from './RustCallList';
import '../styles/projects.css';
import '../styles/operations.css';
import '../styles/rust-call-sites.css';

/**
 * 一个白名单文件：覆盖读数、词法上下文（选择块，所选黑底反白）、种类筛选，
 * 以及调用位置表与右侧的关联侧栏。侧栏在 DOM 里仍排在表格前面，键盘顺序不变。
 */
export function RustCallFileView({
  graph,
  file,
  links,
}: {
  graph: RustCallGraph;
  file: RustSourceCoverage;
  links?: RustCallLinks;
}) {
  const [contextId, setContextId] = useState('all');
  const [kind, setKind] = useState<'all' | RustCallKind>('all');
  const [selection, setSelection] = useState<RustLinkSelection>(null);
  const detailRef = useRef<HTMLElement>(null);
  const associations = links
    ? new Map(links.associations.map((item) => [item.call_id, item]))
    : undefined;
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
          type="button"
          className="rust-context"
          aria-pressed={contextId === 'all'}
          onClick={() => {
            setContextId('all');
            setSelection(null);
          }}
        >
          <strong>全部上下文</strong>
        </button>
        {contexts.map((context) => {
          const parent = context.parent ? contextMap.get(context.parent) : null;
          return (
            <button
              key={context.id}
              type="button"
              className="rust-context"
              aria-pressed={contextId === context.id}
              onClick={() => {
                setContextId(context.id);
                setSelection(null);
              }}
            >
              <strong>
                {rustContextKinds[context.kind]}
                {context.name ? ` · ${context.name}` : ''}
              </strong>
              <span className="rust-context__at">
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
        <span className="project-field project-field--select rust-select">
          <select
            id="rust-call-kind"
            value={kind}
            onChange={(event) => {
              setKind(event.target.value as typeof kind);
              setSelection(null);
            }}
          >
            <option value="all">全部种类</option>
            {Object.entries(rustCallKinds).map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </span>
        <span className="rust-call-filter__count">当前 {calls.length} 处</span>
      </div>
      <div className={links ? 'rust-call-area has-links' : 'rust-call-area'}>
        {links ? (
          <RustCallLinkDetails
            graph={graph}
            path={file.path}
            links={links}
            selection={selection}
            onSelect={setSelection}
            focusRef={detailRef}
          />
        ) : null}
        {calls.length ? (
          <RustCallList
            calls={calls}
            contexts={contextMap}
            associations={associations}
            selection={selection}
            onSelect={(next) => {
              setSelection(next);
              detailRef.current?.focus();
            }}
          />
        ) : (
          <p className="operation-muted rust-call-empty" role="status">
            {fileCalls.length === 0
              ? '此白名单文件没有提取到调用位置；不表示文件无行为，也不覆盖其他文件。'
              : '当前上下文与种类没有匹配位置；其他上下文或种类仍可能有结果。'}
          </p>
        )}
      </div>
    </div>
  );
}
