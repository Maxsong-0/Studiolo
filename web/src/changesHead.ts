/**
 * 变更水位（T-210）：只问「有没有新变化」，不读列表。
 * 页面拿它和手里那一页的水位比，变了才提示重读；一次响应只有库身份和四项水位。
 */
import { getJson } from './sessionHttp';
import { changesThrough, type ChangesThrough } from './changesModel';

export interface ChangesHead {
  vault_uuid: string;
  through: ChangesThrough;
}

const UUID = /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/;

/** 封闭解码：多一个、少一个字段都拒绝；给了期望的库就必须是它。 */
export function changesHead(value: unknown, vault: string | null): ChangesHead {
  const head = value as Partial<ChangesHead> | null;
  if (
    head === null ||
    typeof head !== 'object' ||
    Array.isArray(head) ||
    Object.keys(head).length !== 2 ||
    typeof head.vault_uuid !== 'string' ||
    !UUID.test(head.vault_uuid) ||
    (vault !== null && head.vault_uuid !== vault) ||
    !changesThrough(head.through)
  )
    throw new Error('变更水位响应格式无效，请重新读取。');
  return { vault_uuid: head.vault_uuid, through: head.through };
}

/**
 * 两个水位下的列表是不是同一份。代次号（generation）每次发布都会变，不代表有新条目，不参与比较；
 * 提交序号、观察序号、进程标识（重启后观察序号从头计）任何一个变了，列表就可能多了条目。
 */
export const sameChanges = (a: ChangesThrough, b: ChangesThrough) =>
  a.sequence === b.sequence &&
  a.observation === b.observation &&
  a.epoch === b.epoch;

export async function readChangesHead(
  vault: string | null,
  signal: AbortSignal,
) {
  const query = vault ? `?${new URLSearchParams({ vault_uuid: vault })}` : '';
  return changesHead(
    await getJson(`/api/v1/changes/head${query}`, signal),
    vault,
  );
}
