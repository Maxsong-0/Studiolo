import type { RefObject } from 'react';
import type { RustCallGraph } from '../rustCallSites';
import type { RustCallLinks, RustLinkSelection } from '../rustCallLinks';
import { RustCallAssociationText } from './RustCallAssociationText';
import { RustSourcePosition } from './RustSourcePosition';

export function RustCallLinkDetails({
  graph,
  path,
  links,
  selection,
  onSelect,
  focusRef,
}: {
  graph: RustCallGraph;
  path: string;
  links: RustCallLinks;
  selection: RustLinkSelection;
  onSelect: (selection: RustLinkSelection) => void;
  focusRef: RefObject<HTMLElement | null>;
}) {
  const definitions = graph.contexts.filter(
    (context) => context.kind === 'function' && context.source.path === path,
  );
  const call =
    selection?.kind === 'call'
      ? graph.calls.find((item) => item.id === selection.id)
      : undefined;
  const association = call
    ? links.associations.find((item) => item.call_id === call.id)
    : undefined;
  const definitionId =
    selection?.kind === 'definition' ? selection.id : association?.target;
  const definition = definitions.find((item) => item.id === definitionId);
  const callMap = new Map(graph.calls.map((item) => [item.id, item]));
  const references = definition
    ? links.associations.filter((item) => item.target === definition.id)
    : [];
  return (
    <section
      className="rust-link-details"
      ref={focusRef}
      tabIndex={-1}
      aria-label="Rust 词法关联详情"
    >
      <h3>同文件函数与引用</h3>
      <p className="operation-muted">
        仅为受限词法关联，不是编译器名字解析、类型分派或执行轨迹。
      </p>
      <label htmlFor="rust-function-definition">同文件自由函数定义</label>
      <select
        id="rust-function-definition"
        value={definition?.id ?? ''}
        onChange={(event) =>
          onSelect(
            event.target.value
              ? { kind: 'definition', id: event.target.value }
              : null,
          )
        }
      >
        <option value="">选择定义查看全部引用</option>
        {definitions.map((item) => (
          <option key={item.id} value={item.id}>
            {item.name} · 第 {item.source.line} 行 · 字节列 {item.source.column}
          </option>
        ))}
      </select>
      {!definitions.length ? (
        <p className="operation-muted">此文件没有可查看的自由函数定义。</p>
      ) : null}
      {call ? (
        <div className="rust-selected-call">
          <h4>所选调用：{call.name ?? '表达式目标未提取'}</h4>
          <RustSourcePosition source={call.source} />
          <RustCallAssociationText
            association={association}
            target={definition}
          />
        </div>
      ) : null}
      {definition ? (
        <div
          className="rust-definition-references"
          role="region"
          aria-label="所选函数的同批引用"
        >
          <h4>函数定义：{definition.name}</h4>
          <RustSourcePosition source={definition.source} />
          <p>
            同批词法关联引用：{references.length}{' '}
            处。不受当前上下文或调用种类筛选限制。
          </p>
          {references.length ? (
            <ul aria-label="函数的词法关联引用">
              {references.map((item) => {
                const reference = callMap.get(item.call_id)!;
                return (
                  <li key={item.call_id}>
                    <button
                      className="text-button"
                      onClick={() =>
                        onSelect({ kind: 'call', id: reference.id })
                      }
                      aria-label={`查看引用 ${reference.name}，第 ${reference.source.line} 行字节列 ${reference.source.column}`}
                    >
                      {reference.name}
                    </button>
                    <RustSourcePosition source={reference.source} />
                  </li>
                );
              })}
            </ul>
          ) : (
            <p className="operation-muted">
              本批没有词法关联引用，不表示该函数未被使用。
            </p>
          )}
        </div>
      ) : !call ? (
        <p className="operation-muted">
          选择调用查看关联，或选择自由函数定义反查引用。
        </p>
      ) : null}
    </section>
  );
}
