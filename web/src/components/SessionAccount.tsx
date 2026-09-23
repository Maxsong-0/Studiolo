import { useState } from 'react';
import type { SessionClient } from '../sessionClient';
import type { SessionView } from '../sessionModel';
import { SESSION_ACCOUNT_ATTRIBUTE } from '../sessionStatus';
import { Icon } from './Icon';
import { SessionDialog } from './SessionDialog';
import '../styles/session.css';

/**
 * 顶栏账户入口：账户与会话对话框只从这里开。状态胶囊浮层的「前往账户与会话」
 * 按 SESSION_ACCOUNT_ATTRIBUTE 找到这个按钮、把焦点带过来，不另开一份对话框。
 */
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
        {...{ [SESSION_ACCOUNT_ATTRIBUTE]: '' }}
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
