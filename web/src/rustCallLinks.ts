import type { RustCallGraph } from './rustCallSites';
import { record } from './rustSourceValidation';

export const rustUnresolvedReasons = {
  callee_form: '调用形式不是本片支持的直接裸标识（也不关联泛型调用形式）。',
  context: '调用不在本片允许的词法上下文中。',
  opaque_scope: '属性、宏、glob 导入、extern 或未覆盖语法使所在作用域不透明。',
  shadowed_name: '同名绑定、导入或非函数声明阻断了查找。',
  ambiguous_name: '最近作用域存在多个同名函数候选。',
  generic_target: '最近的唯一函数定义带有泛型参数。',
  not_found: '当前允许的同文件范围内没有找到函数定义。',
} as const;

export type RustUnresolvedReason = keyof typeof rustUnresolvedReasons;
export type RustCallAssociation =
  | {
      call_id: string;
      status: 'linked';
      target: string;
      reason: null;
    }
  | {
      call_id: string;
      status: 'unresolved';
      target: null;
      reason: RustUnresolvedReason;
    };

export interface RustCallLinks {
  version: 1;
  batch_id: string;
  associations: RustCallAssociation[];
}

export type RustLinkSelection = {
  kind: 'call' | 'definition';
  id: string;
} | null;

// 只校验已提供证据的结构和引用；前端没有正文，不能重算内容哈希或名字解析。
export function validateRustCallLinks(
  value: unknown,
  graph: RustCallGraph,
): RustCallLinks {
  const contexts = new Map(
    graph.contexts.map((context) => [context.id, context]),
  );
  const valid =
    record(value) &&
    value.version === 1 &&
    typeof value.batch_id === 'string' &&
    /^[0-9a-f]{64}$/.test(value.batch_id) &&
    Array.isArray(value.associations) &&
    value.associations.length === graph.calls.length &&
    value.associations.every((association: unknown, index: number) => {
      if (!record(association) || association.call_id !== graph.calls[index].id)
        return false;
      if (association.status === 'linked') {
        const target =
          typeof association.target === 'string'
            ? contexts.get(association.target)
            : undefined;
        const name = graph.calls[index].name;
        let caller = contexts.get(graph.calls[index].context);
        let hasFunction = false;
        while (caller) {
          if (['impl', 'trait', 'const', 'static'].includes(caller.kind))
            return false;
          if (caller.kind === 'function') hasFunction = true;
          caller = caller.parent ? contexts.get(caller.parent) : undefined;
        }
        return (
          association.reason === null &&
          graph.calls[index].kind === 'path' &&
          hasFunction &&
          target?.kind === 'function' &&
          target.source.path === graph.calls[index].source.path &&
          typeof name === 'string' &&
          !name.includes('::') &&
          name.replace(/^r#/, '').normalize('NFC') ===
            target.name?.replace(/^r#/, '').normalize('NFC')
        );
      }
      return (
        association.status === 'unresolved' &&
        association.target === null &&
        typeof association.reason === 'string' &&
        Object.hasOwn(rustUnresolvedReasons, association.reason)
      );
    });
  if (!valid)
    throw new Error(
      'Rust 词法关联数据格式、批标识或引用不完整，请重试或检查服务。',
    );
  return value as unknown as RustCallLinks;
}
