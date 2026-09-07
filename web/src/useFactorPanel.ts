import { useCallback, useEffect, useRef, useState } from 'react';
import type { FactorAction, FactorState } from './factorModel';
import type { SessionClient } from './sessionClient';

export function useFactorPanel(client: SessionClient) {
  const [state, setState] = useState<FactorState | null | undefined>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [codes, setCodes] = useState<string[] | null>(null);
  const [formRevision, setFormRevision] = useState(0);
  const lifetime = useRef<AbortController | null>(null);
  const read = useRef<AbortController | null>(null);
  const clear = useCallback(() => {
    lifetime.current?.abort();
    lifetime.current = new AbortController();
    read.current?.abort();
    setCodes(null);
    setState(undefined);
    setError('');
    setNotice('');
    setLoading(false);
    setFormRevision((value) => value + 1);
  }, []);
  const load = useCallback(async () => {
    read.current?.abort();
    const controller = new AbortController();
    read.current = controller;
    setLoading(true);
    setState(undefined);
    setError('');
    try {
      const value = await client.factorState(controller.signal);
      if (!controller.signal.aborted) setState(value);
    } catch {
      if (!controller.signal.aborted)
        setError('暂时无法确认安全操作是否可用，请重新核对。');
    } finally {
      if (!controller.signal.aborted) setLoading(false);
    }
  }, [client]);
  useEffect(() => {
    lifetime.current = new AbortController();
    void load();
    return () => {
      lifetime.current?.abort();
      read.current?.abort();
    };
  }, [load]);
  async function submit(action: FactorAction) {
    const controller = lifetime.current;
    if (!controller || controller.signal.aborted) return;
    setError('');
    setNotice('');
    setCodes(null);
    const outcome = await client.factor(action, controller.signal);
    if (controller !== lifetime.current || controller.signal.aborted) return;
    setFormRevision((value) => value + 1);
    if (outcome.kind === 'failed') {
      setState(undefined);
      setError(outcome.message);
    } else if (outcome.kind === 'step-up') {
      setNotice('二次认证已完成，正在核对当前安全状态。');
      void load();
    } else if (outcome.kind === 'codes') {
      setCodes(outcome.codes);
    }
  }
  function dismissCodes() {
    setCodes(null);
    void load();
  }
  return {
    state,
    loading,
    error,
    notice,
    codes,
    formRevision,
    clear,
    load,
    submit,
    dismissCodes,
  };
}
