import { render, renderHook, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { describe, expect, it } from 'vitest';
import { SessionTime } from '../../components/SessionDialog';
import {
  SessionStatusContext,
  sessionTime,
  useSessionStatus,
} from '../../sessionStatus';
import type { SessionStatus } from '../../sessionStatus';

describe('会话读数的 context', () => {
  it('上面没有 SessionBoundary 时读到 null（胶囊据此报「会话未知」）', () => {
    const { result } = renderHook(() => useSessionStatus());
    expect(result.current).toBeNull();
  });

  it('有 Provider 时原样读到阶段、会话、在途与安全验证提示', () => {
    const status: SessionStatus = {
      phase: 'legacy',
      session: null,
      busy: false,
      factorNotice: null,
    };
    const wrapper = ({ children }: { children: ReactNode }) => (
      <SessionStatusContext.Provider value={status}>
        {children}
      </SessionStatusContext.Provider>
    );
    const { result } = renderHook(() => useSessionStatus(), { wrapper });
    expect(result.current).toBe(status);
  });
});

describe('会话时间', () => {
  it('本机时区 YYYY-MM-DD HH:mm，补零；另给 ISO 串', () => {
    const ms = new Date(2026, 0, 5, 7, 3, 59).getTime();
    expect(sessionTime(ms)).toEqual({
      text: '2026-01-05 07:03',
      iso: new Date(ms).toISOString(),
    });
  });

  it('超出 Date 范围的毫秒值不补时间：返回 null，界面显示「—」并写明原因', () => {
    const outOfRange = 8_640_000_000_000_001;
    expect(sessionTime(outOfRange)).toBeNull();
    render(<SessionTime ms={outOfRange} />);
    const dash = screen.getByText('—');
    expect(dash).toHaveAttribute('title', '时间不可用');
    expect(dash).toHaveTextContent('时间不可用');
    expect(document.querySelector('time')).toBeNull();
  });

  it('能表示的时间渲染成带机读时间的 <time>', () => {
    const ms = new Date(2026, 8, 23, 13, 44).getTime();
    render(<SessionTime ms={ms} />);
    expect(screen.getByText('2026-09-23 13:44')).toHaveAttribute(
      'datetime',
      new Date(ms).toISOString(),
    );
  });
});
