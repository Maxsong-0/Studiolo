import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { readVersion } from '../policyApi';
import type { VersionRecord } from '../policyModel';

/** 一版正文的读取状态。 */
export type VersionBody =
  | { kind: 'loading' }
  | { kind: 'ready'; record: VersionRecord }
  | { kind: 'failed' };

export interface VersionBodies {
  /** 已经要过的各版；没要过的不在表里。 */
  bodies: ReadonlyMap<number, VersionBody>;
  /** 要这一版的正文：已经读到或正在读就不再读；没读成返回 null（再要一次会重读）。 */
  read: (version: number) => Promise<VersionRecord | null>;
  /** 别处已经读到的那一版（如「版本与回滚」里点开的）：直接记下，不重复请求。 */
  store: (record: VersionRecord) => void;
}

/**
 * 策略版本正文的本页缓存（T-198）：在读的版本、能力差异表的对比基准、「从生效中的版本开始改」都从这里取。
 * 版本一经发布，正文和元数据都不会再变，所以同一版只读一次；缓存只活在这个面板里，不放模块级，
 * 整页重挂、换账号都从空的开始。读失败只记「没读成」，由用到它的那一块说明，不弹通知、不重试。
 */
export function useVersionBodies(): VersionBodies {
  const [bodies, setBodies] = useState<ReadonlyMap<number, VersionBody>>(
    () => new Map(),
  );
  // 每一版的那一次读取（在途的和读成了的都留着）；没读成的会去掉，下次再要就重读。
  const reads = useRef(new Map<number, Promise<VersionRecord | null>>());
  // 控制器在挂载时重建（BUG-016）：StrictMode 的装载—卸载—再装载会先 abort 一次；那一次在途的读取
  // 也跟着作废，记号一并清掉，重新装载后才会真的再读。
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    lifetime.current = new AbortController();
    const known = reads.current;
    return () => {
      lifetime.current.abort();
      known.clear();
    };
  }, []);

  const put = useCallback((version: number, body: VersionBody) => {
    setBodies((current) => new Map(current).set(version, body));
  }, []);

  const read = useCallback(
    (version: number) => {
      const known = reads.current.get(version);
      if (known) return known;
      const signal = lifetime.current.signal;
      put(version, { kind: 'loading' });
      const promise = readVersion(version, signal).then(
        (record) => {
          if (signal.aborted) return null;
          put(version, { kind: 'ready', record });
          return record;
        },
        () => {
          if (signal.aborted) return null;
          reads.current.delete(version);
          put(version, { kind: 'failed' });
          return null;
        },
      );
      reads.current.set(version, promise);
      return promise;
    },
    [put],
  );

  const store = useCallback(
    (record: VersionRecord) => {
      reads.current.set(record.meta.version, Promise.resolve(record));
      put(record.meta.version, { kind: 'ready', record });
    },
    [put],
  );

  return useMemo(() => ({ bodies, read, store }), [bodies, read, store]);
}
