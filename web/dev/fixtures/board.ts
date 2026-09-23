/**
 * 协作黑板：`/api/v1/board?project=…` 的工作项与 `/api/v1/board/overview` 的跨项目汇总条目。
 *
 * 画布协作页的五条执行线：重设计线（T-130）、全项目收尾线（T-109）、接入线（T-106、T-108）、
 * M4 接续线（T-092、T-093）、Codex 根统筹（T-046，认领后长时间无进度＝休眠）。
 * T-092 的写域 `web/src/policyApi.ts` 故意落在 T-130 的 `web` 里，构成一处写域重叠。
 * 另有简报站一条今天结束的验收任务，只因窗口内有事件而出现在汇总里。
 *
 * 线上形状以 `web/src/boardModel.ts` 的封闭解码器为准：租约、attempt、回执之间的一致性全部满足。
 */
import { DAY, MINUTE, clockAt } from './clock.ts';
import {
  ATTEMPT,
  FIXTURE_MARKER,
  INSTANCE,
  INSTANCE_TREE,
  POLICY_HASH,
  TASK,
  lineage,
  sha256,
} from './ids.ts';
import type { InstanceKey, TaskKey } from './ids.ts';

export type BoardStatus =
  | 'draft'
  | 'queued'
  | 'claimed'
  | 'running'
  | 'committing'
  | 'cancel_requested'
  | 'needs_review'
  | 'needs_revision'
  | 'needs_reconfirm'
  | 'approved'
  | 'succeeded'
  | 'failed'
  | 'cancelled'
  | 'abandoned';
type Phase = 'completed' | 'failed' | 'cancelled' | 'abandoned';

export const IN_FLIGHT: readonly BoardStatus[] = [
  'claimed',
  'running',
  'cancel_requested',
  'committing',
];
/** 已结束（不再等任何人）：只有这四种；needs_* 与 approved 仍算未结束。 */
export const FINISHED: readonly BoardStatus[] = [
  'succeeded',
  'failed',
  'cancelled',
  'abandoned',
];

export interface TaskDef {
  key: TaskKey;
  title: string;
  project: string;
  adapter: 'claude' | 'codex';
  role: string;
  instruction: string;
  inputPaths: string[];
  paths: string[];
  criteria: string[];
  creator: InstanceKey;
  holder: InstanceKey;
  fence: number;
  status: BoardStatus;
  /** 冻结时的策略版本；与当前 effective 不同就是「按旧版规则执行」。 */
  policyHash: string;
  lease: (time: FixtureTime) => number;
  progress: {
    tools: number;
    edits: number;
    last: (time: FixtureTime) => number;
  };
  /** 候选产物：代码补丁（candidate.patch ＋ patch.json）或一份结果报告。 */
  candidate: 'patch' | 'report' | null;
  stageReason: string | null;
  handoff: Record<string, unknown> | null;
}

/**
 * `now`：此刻，在跑任务的租约据此永远有效；`anchor`：假数据启动时刻，当天排期、进度时间与
 * 已结束的租约都挂在它上面，和事件流保持一致（开着的开发服务器里不会越跑越错位）。
 */
export interface FixtureTime {
  now: number;
  anchor: number;
}

/** 回执：线上形状与 `boardModel.ts` 的 `receipt` 一致。 */
export type Receipt = Record<string, unknown> & { status: string };

const STUDIOLO = '项目/Studiolo';
const leaseAhead =
  (minutes: number) =>
  ({ now }: FixtureTime) =>
    now + minutes * MINUTE;
const endedAt =
  (hour: number) =>
  ({ anchor }: FixtureTime) =>
    clockAt(anchor, hour) + 5 * MINUTE;
const at =
  (hour: number) =>
  ({ anchor }: FixtureTime) =>
    clockAt(anchor, hour);

export const TASK_DEFS: TaskDef[] = [
  {
    key: 'T046',
    title: 'T-046 M2/M3 整体收尾',
    project: STUDIOLO,
    adapter: 'codex',
    role: 'coordinator',
    instruction: '按 M2、M3 收尾台账逐项核对，遗留项转入 TASKS 并注明去向。',
    inputPaths: ['docs/m2/m2-m3-closeout.md'],
    paths: ['crates/vault-broker'],
    criteria: ['M2、M3 收尾台账逐项核对', '遗留项转入 TASKS 并注明去向'],
    creator: 'R5',
    holder: 'R5',
    fence: 3,
    status: 'claimed',
    policyHash: POLICY_HASH.v1,
    lease: leaseAhead(9),
    progress: { tools: 3, edits: 0, last: ({ anchor }) => anchor - 4 * DAY },
    candidate: null,
    stageReason: null,
    handoff: null,
  },
  {
    key: 'T092',
    title: 'T-092 人侧 M4 L3 部署',
    project: STUDIOLO,
    adapter: 'codex',
    role: 'builder',
    instruction: '人侧 M4 部署：策略发布走 One-time Code，403 就地弹认证窗口。',
    inputPaths: ['docs/m4/README.md'],
    paths: ['web/src/policyApi.ts'],
    criteria: [
      '策略发布走 One-time Code，403 就地弹认证窗口',
      '公网实测策略页刷新不报错',
    ],
    creator: 'R2',
    holder: 'C2a',
    fence: 2,
    status: 'approved',
    policyHash: POLICY_HASH.v3,
    lease: endedAt(14.35),
    progress: { tools: 57, edits: 6, last: at(14.3) },
    candidate: 'patch',
    stageReason: '执行体提交候选，已审阅通过，写域尚未释放',
    handoff: null,
  },
  {
    key: 'T093',
    title: 'T-093 M4 异机灾备与恢复演练',
    project: STUDIOLO,
    adapter: 'codex',
    role: 'operator',
    instruction:
      '准备离线主机的恢复脚本；真人 L3 恢复与 RTO/RPO 演练需要主人在场。',
    inputPaths: ['docs/m4/README.md'],
    paths: ['deploy/human/offline'],
    criteria: [
      '异机恢复脚本可在离线主机执行',
      '真人 L3 恢复与 RTO/RPO 实测记录入档',
    ],
    creator: 'R2',
    holder: 'C2b',
    fence: 1,
    status: 'needs_revision',
    policyHash: POLICY_HASH.v3,
    lease: endedAt(10.1),
    progress: { tools: 61, edits: 9, last: at(10.05) },
    candidate: 'patch',
    stageReason: '预算耗尽，候选留存；演练需要主人在场',
    handoff: {
      goal: '主人在场完成真人 L3 恢复演练，并记录 RTO/RPO',
      constraints: [
        { rule_id: 'studiolo-l3-drill', version_hash: POLICY_HASH.v3 },
      ],
      decided: ['离线主机使用 deploy/human/offline 下的脚本'],
      undecided: ['演练时间窗口'],
      required_files: [
        { path: 'deploy/human/offline/README.md', revision: 'c9b687f' },
      ],
      acceptance_criteria: ['RTO 不超过 30 分钟', 'RPO 不超过 24 小时'],
      forbidden: ['不在生产主机上演练'],
    },
  },
  {
    key: 'B021',
    title: 'T-021 简报站 v0.3.1 公网验收',
    project: '项目/Brief 简报站',
    adapter: 'claude',
    role: 'verifier',
    instruction: '公网逐页验收，结果写成报告；只读，不改任何文件。',
    inputPaths: ['项目/Brief 简报站/05-版本/roadmap.md'],
    paths: [],
    criteria: ['公网首页与 17 个分类页均可访问', '地铁线路图在窄屏自适应'],
    creator: 'R3',
    holder: 'C3b',
    fence: 1,
    status: 'succeeded',
    policyHash: POLICY_HASH.v3,
    lease: endedAt(10.9),
    progress: { tools: 34, edits: 2, last: at(10.85) },
    candidate: 'report',
    stageReason: '验收报告已提交',
    handoff: null,
  },
  {
    key: 'T106',
    title: 'T-106 安装文档补运行包冻结',
    project: STUDIOLO,
    adapter: 'claude',
    role: 'writer',
    instruction: '在 docs/m3 补运行包冻结与生产安装流程，去掉写死的白名单。',
    inputPaths: ['docs/m3/server-agent.md'],
    paths: ['docs/m3'],
    criteria: [
      'docs/m3/server-agent.md 补运行包冻结与生产安装流程',
      '文档里不再写死白名单',
    ],
    creator: 'R4',
    holder: 'C4a',
    fence: 1,
    status: 'needs_review',
    policyHash: POLICY_HASH.v3,
    lease: endedAt(15.1),
    progress: { tools: 88, edits: 19, last: at(15.05) },
    candidate: 'patch',
    stageReason: '执行体提交候选，等待审阅与代码落地',
    handoff: null,
  },
  {
    key: 'T108',
    title: 'T-108 副本扩容纳入 Semiment-Docs',
    project: STUDIOLO,
    adapter: 'claude',
    role: 'builder',
    instruction:
      '副本总量上限 128→512 MiB 并纳入 项目/Semiment-Docs；附件每次请求只核大小。',
    inputPaths: ['docs/contracts/agent-supply.md'],
    paths: ['scripts', 'crates/supply/src/replica'],
    criteria: [
      '副本总量上限 128→512 MiB，纳入 项目/Semiment-Docs',
      '附件每次请求只核大小，不重读正文',
      'scripts/test-replica-boundaries.py 全部通过',
    ],
    creator: 'R4',
    holder: 'C4b',
    fence: 1,
    status: 'needs_review',
    policyHash: POLICY_HASH.v3,
    lease: endedAt(15.6),
    progress: { tools: 143, edits: 27, last: at(15.55) },
    candidate: 'patch',
    stageReason: '执行体提交候选，等待审阅与代码落地',
    handoff: null,
  },
  {
    key: 'T109',
    title: 'T-109 全项目未完成项盘点',
    project: STUDIOLO,
    adapter: 'claude',
    role: 'auditor',
    instruction: '逐个项目核对 04-协调/TASKS.md 与日志，列出未完成项并回写。',
    inputPaths: ['项目/Studiolo/04-协调/TASKS.md'],
    paths: ['04-协调'],
    criteria: [
      '22 个可读项目逐一核对，未完成项回写各自 TASKS',
      '每项附来源行号，无法判断的写明原因',
    ],
    creator: 'R3',
    holder: 'C3',
    fence: 1,
    status: 'running',
    policyHash: POLICY_HASH.v3,
    lease: leaseAhead(6),
    progress: { tools: 96, edits: 14, last: at(18.5) },
    candidate: null,
    stageReason: null,
    handoff: null,
  },
  {
    key: 'T130',
    title: 'T-130 前端全面重设计',
    project: STUDIOLO,
    adapter: 'claude',
    role: 'builder',
    instruction:
      '按 docs/design/t130-design-system.md 重做前端视觉：瑞士网格为骨、工程图纸为表达；接口与行为不变。',
    inputPaths: ['docs/design/t130-design-system.md'],
    paths: ['web'],
    criteria: [
      '驾驶舱、协作页、⌘K 面板在 1440×900 下与画布主稿一致',
      '全部接口解码器保持封闭，npm run build 与 vitest 全绿',
      'BUG-011、BUG-015、BUG-016 三处修法及其测试保留并通过',
    ],
    creator: 'R1',
    holder: 'C1',
    fence: 1,
    status: 'running',
    policyHash: POLICY_HASH.v3,
    lease: leaseAhead(4),
    progress: { tools: 212, edits: 38, last: at(19.95) },
    candidate: null,
    stageReason: null,
    handoff: {
      goal: '按 T-130 规范完成 A 轮地基：令牌、外壳、网格与假数据模式',
      constraints: [
        { rule_id: 'studiolo-ui-csp', version_hash: POLICY_HASH.v3 },
      ],
      decided: ['A2 管骨架，A4 管表达', '唯一强调色只表示等你或出问题'],
      undecided: ['⌘K 面板动作分组的最终顺序'],
      required_files: [
        { path: 'docs/design/t130-design-system.md', revision: '91981de' },
      ],
      acceptance_criteria: ['A 轮五个任务各自回执 completed'],
      forbidden: ['不加载任何远程资源', '不写 style 标签', '不放宽任何解码器'],
    },
  },
];

export const TASK_BY_KEY = new Map(TASK_DEFS.map((def) => [def.key, def]));
export const taskKeyOf = (id: string): TaskKey | undefined =>
  TASK_DEFS.find((def) => TASK[def.key] === id)?.key;

/** 可变部分：状态、attempt 结束时的阶段、回执、是否已落地。 */
export interface TaskState {
  status: BoardStatus;
  phase: Phase;
  receipt: Receipt | null;
  landed: boolean;
}

export function initialTaskState(def: TaskDef): TaskState {
  return {
    status: def.status,
    phase: 'completed',
    receipt: def.key === 'B021' ? briefReceipt(def) : null,
    landed: def.key === 'T092',
  };
}

const pointer = (key: TaskKey, name: string) => ({
  path: `sources/attempts/${TASK[key]}/${ATTEMPT[key]}/${name}`,
  revision: sha256(`revision ${key} ${name}`),
  sha256: sha256(`bytes ${key} ${name}`),
});

/** candidate.patch 的 SHA-256：落地命令必须原样带上它。 */
export const patchSha256 = (key: TaskKey) =>
  sha256(`bytes ${key} candidate.patch`);

function candidateReceipt(def: TaskDef) {
  if (!def.candidate) return null;
  return {
    manifest_path: `sources/attempts/${TASK[def.key]}/${ATTEMPT[def.key]}/manifest.json`,
    manifest_revision: sha256(`manifest ${def.key}`),
    artifacts:
      def.candidate === 'patch'
        ? [pointer(def.key, 'candidate.patch'), pointer(def.key, 'patch.json')]
        : [pointer(def.key, 'result.md')],
  };
}

function briefReceipt(def: TaskDef): Receipt {
  const report = pointer(def.key, 'result.md');
  return {
    status: 'completed',
    artifacts: [report],
    conflicts: [],
    unlanded: [],
    acceptance: def.criteria.map((criterion) => ({
      criterion,
      result: 'pass',
      evidence: [report],
    })),
    remaining: [],
    elapsed_ms: 1_980_000,
    tool_calls: 34,
    learned: ['窄屏下线路图改用横向滚动比缩放更可读'],
    closeout: {
      learned: ['窄屏下线路图改用横向滚动比缩放更可读'],
      adr_required: false,
      rule_change_required: false,
      destinations: ['项目/Brief 简报站/03-记忆'],
    },
  };
}

const node = (key: InstanceKey) => ({
  id: INSTANCE[key],
  agent_type: INSTANCE_TREE[key].type,
});

/** 创建实例与持有实例各一条链（去重、按 ID 排序），与后端 `board/http.rs` 一致。 */
function instanceLineages(def: TaskDef) {
  return [...new Set([def.creator, def.holder])]
    .sort((left, right) => (INSTANCE[left] < INSTANCE[right] ? -1 : 1))
    .map((key) => lineage(key).map(node));
}

/** `/api/v1/board` 的一条工作项（线上 JSON）。 */
export function boardWire(
  def: TaskDef,
  state: TaskState,
  time: FixtureTime,
  effectiveHash: string,
) {
  const active = IN_FLIGHT.includes(state.status);
  const attemptId = ATTEMPT[def.key];
  const holder = INSTANCE[def.holder];
  const snapshot = sha256(`snapshot ${def.key}`);
  const stage = {
    id: sha256(`stage id ${def.key}`),
    sha256: sha256(`stage bytes ${def.key}`),
  };
  return {
    task: {
      task_id: TASK[def.key],
      spec: {
        role: def.role,
        adapter: def.adapter,
        input_paths: def.inputPaths,
        allow_web: [],
        allow_db: [],
        retry_policy: {
          max_attempts: 2,
          backoff_seconds: 30,
          retry_on: ['launch_failed'],
        },
        timeout: 5400,
        project: def.project,
        parent: null,
        title: def.title,
        instruction: def.instruction,
      },
      status: state.status,
      active_attempt: active ? attemptId : null,
      last_attempt: active ? null : attemptId,
      input_snapshot: snapshot,
    },
    board: {
      task_id: TASK[def.key],
      definition: {
        acceptance_criteria: def.criteria,
        files_owned: def.paths,
        max_children: 8,
        policy_hash: def.policyHash,
        parent: null,
        artifact_target: null,
      },
      depth: 0,
      created_by_instance: INSTANCE[def.creator],
      handoff: def.handoff,
      receipt: state.receipt,
    },
    uses_old_policy: def.policyHash !== effectiveHash,
    holder,
    progress: {
      tool_calls: def.progress.tools,
      edits: def.progress.edits,
      last_progress_ms: def.progress.last(time),
    },
    attempt: {
      attempt_id: attemptId,
      task_id: TASK[def.key],
      fence: def.fence,
      phase: active ? state.status : state.phase,
      worker_id: holder,
      snapshot_id: snapshot,
      lease_until_ms: def.lease(time),
      runner_ref_json: active
        ? { runner: 'real', workspace: `${FIXTURE_MARKER}-${def.key}` }
        : null,
      terminal: active
        ? null
        : {
            report_json: { attempt_id: attemptId, fence: def.fence },
            artifacts_json: stage,
          },
    },
    candidate_stages: def.stageReason
      ? [
          {
            attempt_id: attemptId,
            fence: def.fence,
            pointer: stage,
            reason: def.stageReason,
          },
        ]
      : [],
    candidate_receipt: candidateReceipt(def),
    instance_lineages: instanceLineages(def),
  };
}

/** `/api/v1/board/overview` 的一条（字段按设计规范第 10 节，一个不多一个不少）。 */
export function overviewItem(
  def: TaskDef,
  state: TaskState,
  time: FixtureTime,
) {
  const active = IN_FLIGHT.includes(state.status);
  return {
    id: TASK[def.key],
    title: def.title,
    project: def.project,
    status: state.status,
    parent: null,
    adapter: def.adapter,
    holder: INSTANCE[def.holder],
    holder_lineage: lineage(def.holder).map(node),
    paths: def.paths,
    attempt: {
      phase: active ? state.status : state.phase,
      worker: INSTANCE[def.holder],
      active,
      lease_until_ms: def.lease(time),
    },
    progress: {
      tools: def.progress.tools,
      edits: def.progress.edits,
      last_progress_ms: def.progress.last(time),
    },
    receipt_status: state.receipt?.status ?? null,
  };
}
