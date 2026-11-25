/**
 * `/api/v1/board/presence`：连接器推来的本机会话（T-237）。三种状态各有一条，字段带全、只带必带的各有一条。
 */
import { HOUR, iso } from './clock.ts';

export function presenceBody(now: number) {
  return {
    since: iso(now - 6 * HOUR),
    expires_in_secs: 300,
    sessions: [
      {
        source: 'homelab-status',
        vendor: 'claude_code',
        project: '项目/HomeLab-Center-Docs',
        branch: 'claude/t-092-coding',
        state: 'waiting',
        step: 3,
        total: 8,
        last_action: 'ask',
        idle_secs: 240,
        age_secs: 12,
      },
      {
        source: 'homelab-status',
        vendor: 'claude_code',
        project: '项目/Studiolo-Docs',
        branch: 'claude/t-237-connector',
        state: 'running',
        step: 5,
        last_action: 'edit',
        idle_secs: 4,
        age_secs: 12,
      },
      {
        source: 'homelab-status',
        vendor: 'codex',
        project: '项目/Studiolo-Docs',
        state: 'idle',
        age_secs: 12,
      },
    ],
  };
}
