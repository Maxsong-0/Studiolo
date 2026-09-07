import { StrictMode } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect } from 'vitest';
import { SessionBoundary } from '../SessionBoundary';
import type { SessionClient } from '../sessionClient';

export async function openFactorAccount(client?: SessionClient) {
  const user = userEvent.setup();
  const rendered = render(
    <StrictMode>
      <SessionBoundary client={client}>
        {(account) => (
          <>
            {account}
            <p>已授权内容</p>
          </>
        )}
      </SessionBoundary>
    </StrictMode>,
  );
  await user.click(await screen.findByRole('button', { name: '账户与会话' }));
  await waitFor(() =>
    expect(screen.queryByText('正在核对安全操作…')).not.toBeInTheDocument(),
  );
  return { user, ...rendered };
}
