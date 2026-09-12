import { rustUnresolvedReasons } from '../rustCallLinks';
import type { RustCallAssociation } from '../rustCallLinks';
import type { RustCallContext } from '../rustCallSites';
import { RustSourcePosition } from './RustSourcePosition';

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
