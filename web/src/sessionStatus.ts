import { createContext, useContext } from 'react';
import type { SessionInfo, SessionView } from './sessionModel';

/**
 * 顶栏状态胶囊要的会话读数（T-130 设计规范第 5 节）：只是 SessionClient 快照里的四项，
 * 由 SessionBoundary 在放行内容的那一支（legacy/ready）用 Provider 发下去。
 * 这里不保存 CSRF、主人范围或任何因子输入；胶囊只读，不能借它发请求。
 */
export interface SessionStatus {
  phase: SessionView['phase'];
  session: SessionInfo | null;
  busy: boolean;
  factorNotice: string | null;
}

/** null＝上面没有 SessionBoundary：会话信息不可用，胶囊按「会话未知」处理，绝不当成正常。 */
export const SessionStatusContext = createContext<SessionStatus | null>(null);

export function useSessionStatus(): SessionStatus | null {
  return useContext(SessionStatusContext);
}

/**
 * 顶栏账户入口（SessionAccount 的按钮）上的标记：胶囊浮层的「前往账户与会话」据此把焦点带过去，
 * 不在胶囊里另开账户对话框，免得两处入口各持一份状态。
 */
export const SESSION_ACCOUNT_ATTRIBUTE = 'data-session-account';

const pad = (value: number) => String(value).padStart(2, '0');

/**
 * 会话时间：本机时区 `YYYY-MM-DD HH:mm`，另给 ISO 串放进 `<time dateTime>`。
 * 解码器只保证是非负安全整数，可能超出 Date 的范围：那种返回 null，由调用方写明「时间不可用」。
 */
export function sessionTime(ms: number): { text: string; iso: string } | null {
  const date = new Date(ms);
  if (!Number.isFinite(date.getTime())) return null;
  const text = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(
    date.getDate(),
  )} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
  return { text, iso: date.toISOString() };
}
