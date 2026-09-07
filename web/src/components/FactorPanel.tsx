import { useEffect } from 'react';
import type { RefObject } from 'react';
import type { SessionClient } from '../sessionClient';
import { useFactorPanel } from '../useFactorPanel';
import { FactorForms } from './FactorForms';
import { RecoveryDelivery } from './RecoveryDelivery';
import { Icon } from './Icon';

export function FactorPanel({
  client,
  busy,
  cleanup,
}: {
  client: SessionClient;
  busy: boolean;
  cleanup: RefObject<(() => void) | null>;
}) {
  const panel = useFactorPanel(client);
  useEffect(() => {
    cleanup.current = panel.clear;
    return () => {
      cleanup.current = null;
    };
  }, [cleanup, panel.clear]);
  const warning = client.getSnapshot().factorNotice;
  return (
    <section className="factor-panel" aria-labelledby="factor-title">
      <div className="factor-heading">
        <h3 id="factor-title">
          <Icon name="shield" />
          安全验证
        </h3>
        <button
          className="text-button"
          disabled={busy || panel.loading || !!panel.codes}
          onClick={() => void panel.load()}
        >
          核对安全状态
        </button>
      </div>
      {warning ? (
        <p className="factor-warning" role="status">
          {warning}
        </p>
      ) : null}
      {panel.loading ? <p role="status">正在核对安全操作…</p> : null}
      {panel.error ? (
        <p className="factor-warning" role="alert">
          {panel.error}
        </p>
      ) : null}
      {panel.notice ? <p role="status">{panel.notice}</p> : null}
      {panel.state === null ? (
        <p>当前入口未启用二次认证和验证器恢复操作。</p>
      ) : null}
      {panel.codes ? (
        <RecoveryDelivery codes={panel.codes} onClose={panel.dismissCodes} />
      ) : panel.state ? (
        <>
          {panel.state.pending ? (
            <p className="factor-warning">
              有认证决定尚待处理。状态不会确认先前操作的结果；请勿重复使用刚才的输入。
            </p>
          ) : null}
          <FactorForms
            key={panel.formRevision}
            state={panel.state}
            busy={busy}
            onSubmit={(action) => void panel.submit(action)}
            onCancel={panel.clear}
          />
        </>
      ) : null}
    </section>
  );
}
