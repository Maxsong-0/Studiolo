import { rustUnresolvedReasons } from '../rustCallLinks';
import type { RustCallAssociation } from '../rustCallLinks';
import type { RustCallContext } from '../rustCallSites';
import { RustSourcePosition } from './RustSourcePosition';
import '../styles/operations.css';
import '../styles/rust-call-links.css';

/**
 * 一处调用的词法关联：关联到同文件函数定义时给出定义位置；未解析时按闭集原因说明，不附猜测目标；
 * 旧服务没有关联数据时明说。未解析是中性信息，不用红色。
 */
export function RustCallAssociationText({
  association,
  target,
}: {
  association?: RustCallAssociation;
  target?: RustCallContext;
}) {
  if (!association)
    return (
      <p className="operation-muted">旧服务未提供词法关联，目标未解析。</p>
    );
  if (association.status === 'unresolved')
    return (
      <p className="rust-link-unresolved">
        未解析：{rustUnresolvedReasons[association.reason]}
      </p>
    );
  return (
    <div className="rust-linked-definition">
      <p>
        词法关联的同文件函数定义：<strong>{target?.name}</strong>
      </p>
      {target ? <RustSourcePosition source={target.source} /> : null}
    </div>
  );
}
