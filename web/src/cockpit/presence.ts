/**
 * 驾驶舱「本机会话」一组的换算（T-237，纯函数）：排序、人话、数几个在跑几个等你。
 * 不计入「等你处理」的巨号和角标：那些会话要去它自己的应用里点，Studiolo 里处理不了。
 */
import type {
  Presence,
  PresenceAction,
  PresenceSession,
  PresenceState,
  PresenceVendor,
} from '../presenceModel';
import { projectName } from './model';

/** 最多列几条，其余只写条数。 */
export const PRESENCE_ROWS = 8;

const VENDORS: Record<PresenceVendor, string> = {
  claude_code: 'Claude Code',
  codex: 'Codex',
  agy: 'agy',
};
const STATES: Record<PresenceState, string> = {
  running: '在跑',
  waiting: '等你',
  idle: '做完或等你回话',
};
const ACTIONS: Record<PresenceAction, string> = {
  read: '在读',
  edit: '在改',
  run: '跑命令',
  web: '查网页',
  ask: '问人',
  dispatch: '派活',
  other: '其它动作',
};
/** 停下等你的排最前，其次在跑的。 */
const ORDER: Record<PresenceState, number> = {
  waiting: 0,
  running: 1,
  idle: 2,
};

export interface PresenceRow {
  key: string;
  state: PresenceState;
  vendor: string;
  /** 项目和分支。 */
  where: string;
  /** 完整的项目路径和来源，放进提示文字。 */
  title: string;
  /** 做到第几条、最近动作、多久没动；都没带就是空串。 */
  detail: string;
  status: string;
}

export interface PresenceGroup {
  rows: PresenceRow[];
  hidden: number;
  running: number;
  waiting: number;
}

/** 多久没动：一分钟以内不写，写成「N 分钟没动」「N 小时没动」。 */
function idleText(secs: number): string | null {
  const minutes = Math.floor(secs / 60);
  if (minutes < 1) return null;
  if (minutes < 60) return `${minutes} 分钟没动`;
  return `${Math.floor(minutes / 60)} 小时没动`;
}

function stepText({ step, total }: PresenceSession): string | null {
  if (step === undefined) return null;
  return total === undefined ? `第 ${step} 条` : `第 ${step}/${total} 条`;
}

function detail(session: PresenceSession): string {
  return [
    stepText(session),
    session.lastAction === undefined ? null : ACTIONS[session.lastAction],
    session.idleSecs === undefined ? null : idleText(session.idleSecs),
  ]
    .filter((part) => part !== null)
    .join(' · ');
}

export function presenceGroup(presence: Presence): PresenceGroup {
  const ordered = presence.sessions
    .map((session, index) => ({ session, index }))
    .sort(
      (a, b) =>
        ORDER[a.session.state] - ORDER[b.session.state] || a.index - b.index,
    );
  const count = (state: PresenceState) =>
    presence.sessions.filter((session) => session.state === state).length;
  return {
    rows: ordered.slice(0, PRESENCE_ROWS).map(({ session, index }) => {
      const name = projectName(session.project, []);
      return {
        // 服务端不给会话编号：同一份应答里按它的次序认。
        key: `${index}`,
        state: session.state,
        vendor: VENDORS[session.vendor],
        where: session.branch ? `${name} · ${session.branch}` : name,
        title: `${session.project}（${session.source} 推来）`,
        detail: detail(session),
        status: STATES[session.state],
      };
    }),
    hidden: Math.max(0, ordered.length - PRESENCE_ROWS),
    running: count('running'),
    waiting: count('waiting'),
  };
}
