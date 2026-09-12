import { rustCallKinds, rustContextKinds } from '../rustCallSites';
import type { RustCallContext, RustCallSite } from '../rustCallSites';
import type { RustCallAssociation, RustLinkSelection } from '../rustCallLinks';
import { RustSourcePosition } from './RustSourcePosition';
import { RustCallAssociationText } from './RustCallAssociationText';

export function RustCallList({
  calls,
  contexts,
  associations,
  selection,
  onSelect,
}: {
  calls: RustCallSite[];
  contexts: Map<string, RustCallContext>;
  associations?: Map<string, RustCallAssociation>;
  selection: RustLinkSelection;
  onSelect: (selection: RustLinkSelection) => void;
}) {
  return (
    <ol className="rust-call-list" aria-label="Rust 调用位置">
      {calls.map((call) => {
        const context = contexts.get(call.context)!;
        const association = associations?.get(call.id);
        return (
          <li key={call.id}>
            <strong>{call.name ?? '表达式目标未提取'}</strong>
            <p>
              {rustCallKinds[call.kind]}
              {!association ? ' · 目标未解析' : ''}
            </p>
            <p>
              最近词法上下文：{context.name ?? rustContextKinds[context.kind]}
            </p>
            <RustSourcePosition source={call.source} />
            {association ? (
              <>
                <RustCallAssociationText
                  association={association}
                  target={
                    association.target
                      ? contexts.get(association.target)
                      : undefined
                  }
                />
                <button
                  className="text-button"
                  aria-pressed={
                    selection?.kind === 'call' && selection.id === call.id
                  }
                  onClick={() => onSelect({ kind: 'call', id: call.id })}
                  aria-label={`查看调用 ${call.name ?? '表达式'} 的词法关联，第 ${call.source.line} 行字节列 ${call.source.column}`}
                >
                  查看关联与引用
                </button>
              </>
            ) : null}
          </li>
        );
      })}
    </ol>
  );
}
