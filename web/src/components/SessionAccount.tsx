import { useState } from 'react';
import type { SessionClient } from '../sessionClient';
import type { SessionView } from '../sessionModel';
import { Icon } from './Icon';
import { SessionDialog } from './SessionDialog';

export function SessionAccount({
  client,
  state,
}: {
  client: SessionClient;
  state: SessionView;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        className="outline-button account-button"
        onClick={() => setOpen(true)}
        aria-haspopup="dialog"
        aria-label="账户与会话"
      >
        <Icon name="account" />
        <span>账户</span>
      </button>
      {open && state.session ? (
        <SessionDialog
          client={client}
          current={state.session}
          busy={state.busy}
          onClose={() => setOpen(false)}
        />
      ) : null}
    </>
  );
}
