import type { RustCallGraph } from '../rustCallSites';
import type { RustCallLinks } from '../rustCallLinks';
import { RustSourcePosition } from './RustSourcePosition';
import '../styles/rust-call-links.css';

/** 文本降级里的反查：一个函数定义的同批词法关联引用，只读，不含按钮。 */
export function RustCallReferencesText({
  target,
  graph,
  links,
}: {
  target: string;
  graph: RustCallGraph;
  links: RustCallLinks;
}) {
  const callMap = new Map(graph.calls.map((call) => [call.id, call]));
  const references = links.associations.filter(
    (item) => item.target === target,
  );
  return (
    <div className="rust-references-text">
      <p>
        同批词法关联引用：{references.length} 处。零引用不表示该函数未被使用。
      </p>
      {references.length ? (
        <ul>
          {references.map((item) => {
            const call = callMap.get(item.call_id)!;
            return (
              <li key={item.call_id}>
                {call.name}
                <RustSourcePosition source={call.source} />
              </li>
            );
          })}
        </ul>
      ) : null}
    </div>
  );
}
