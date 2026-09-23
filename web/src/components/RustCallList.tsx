import { rustCallKinds, rustContextKinds } from '../rustCallSites';
import type { RustCallContext, RustCallSite } from '../rustCallSites';
import type { RustCallAssociation, RustLinkSelection } from '../rustCallLinks';
import { Button } from '../ui';
import { RustSourcePosition } from './RustSourcePosition';
import { RustCallAssociationText } from './RustCallAssociationText';
import '../styles/rust-call-sites.css';
import '../styles/rust-call-links.css';

/**
 * 调用位置表：每行是等宽「行:字节列」坐标、调用名与语法种类文字、最近词法上下文，
 * 有词法关联时再加关联结果与「查看关联与引用」。坐标只给眼睛看（表头写明是 UTF-8 字节列），
 * 读屏读每行里完整的来源句与每格自带的标签；正在侧栏里查看的那一行，坐标块黑底反白。
 */
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
  const linked = associations !== undefined;
  return (
    <div className={linked ? 'rust-call-table has-links' : 'rust-call-table'}>
      <div className="rust-call-table__head" aria-hidden="true">
        <span>行:UTF-8 字节列</span>
        <span>调用 · 语法种类</span>
        <span>最近词法上下文</span>
        {linked ? (
          <>
            <span>词法关联</span>
            <span />
          </>
        ) : null}
      </div>
      <ol className="rust-call-list" aria-label="Rust 调用位置">
        {calls.map((call) => {
          const context = contexts.get(call.context)!;
          const association = associations?.get(call.id);
          const pressed =
            selection?.kind === 'call' && selection.id === call.id;
          return (
            <li
              key={call.id}
              className="rust-call"
              data-selected={pressed ? 'true' : undefined}
            >
              <span className="rust-call__at" aria-hidden="true">
                {call.source.line}:{call.source.column}
              </span>
              <div className="rust-call__site">
                <strong className="rust-call__name">
                  {call.name ?? '表达式目标未提取'}
                </strong>
                <p className="rust-call__kind">
                  {rustCallKinds[call.kind]}
                  {!association ? ' · 目标未解析' : ''}
                </p>
                <RustSourcePosition source={call.source} />
              </div>
              <p className="rust-call__context">
                <span className="rust-cell-label">最近词法上下文：</span>
                {context.name ?? rustContextKinds[context.kind]}
              </p>
              {association ? (
                <>
                  <div className="rust-call__link">
                    <RustCallAssociationText
                      association={association}
                      target={
                        association.target
                          ? contexts.get(association.target)
                          : undefined
                      }
                    />
                  </div>
                  <Button
                    variant="text"
                    size="sm"
                    className="rust-call__inspect"
                    aria-pressed={pressed}
                    onClick={() => onSelect({ kind: 'call', id: call.id })}
                    aria-label={`查看调用 ${call.name ?? '表达式'} 的词法关联，第 ${call.source.line} 行字节列 ${call.source.column}`}
                  >
                    查看关联与引用
                  </Button>
                </>
              ) : null}
            </li>
          );
        })}
      </ol>
    </div>
  );
}
