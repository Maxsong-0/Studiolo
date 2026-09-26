/**
 * 策略平面：`/api/v1/policy`、`/desired`、`/versions`、`/versions/{n}`，以及写入时的校验报告。
 *
 * 初始状态：现行四版（v1 规则 CLAUDE.md、v2 规则 通用工作规则.md、v3 Claude 能力集、v4 Codex 能力集，
 * 规则是 ADR-026 D6 的首批两条）；Claude 能力集的 read.rules_whitelist 列了这两条规则，Codex 的能力集
 * 发布于该字段出现之前、没有这个字段（缺字段即零规则，D4），规则视图的两种状态都看得到。
 * 待处理 desired 一条——全项目收尾线的 Claude 实例为 T-109 盘点申请扩大读取范围（三项扩大，红色）；
 * 吊销三级：一个越界的旧 Codex 实例（实例级）、消费端类型（类型级）生效中，全局未吊销。
 *
 * 校验规则按 `crates/policy/src/validate.rs` 的同一套口径简化实现：命名规则、JSON 形状、
 * 能力集扩大项/收窄项（含规则读名单）、规则母本删除约束性语句的候选扩大项。形状以 `policyModel.ts` 为准。
 */
import { clockAt, iso } from './clock.ts';
import { DESIRED_ID, INSTANCE, POLICY_HASH, sha256 } from './ids.ts';

export type Kind = 'rules' | 'skills' | 'capability_set';
type Expansion = {
  path: string;
  before: string | null;
  after: string | null;
  note: string;
};
type DiffEntry = {
  op: 'added' | 'removed' | 'changed';
  path: string;
  before: string | null;
  after: string | null;
};

export interface Report {
  ok: boolean;
  kind: Kind;
  subject: string;
  retire: boolean;
  problems: { path: string; message: string }[];
  diff: {
    format: string;
    entries: DiffEntry[];
    added: number;
    removed: number;
    changed: number;
    truncated: boolean;
  };
  expansions: Expansion[];
  narrowings: Expansion[];
  base_effective_hash: string;
  current_version: number | null;
  body_sha256: string;
}

export interface DesiredRecord {
  id: string;
  kind: Kind;
  subject: string;
  body: string | null;
  body_sha256: string;
  submitter_kind: 'owner' | 'agent';
  agent_type: string | null;
  agent_instance: string | null;
  submitted_at: string;
  status:
    | 'pending'
    | 'validated'
    | 'rejected'
    | 'activated'
    | 'superseded'
    | 'withdrawn';
  report: Report | null;
  validated_at: string | null;
  activated_version: number | null;
}

export interface VersionMeta {
  version: number;
  kind: Kind;
  subject: string;
  retired: boolean;
  body_sha256: string;
  effective_hash: string;
  previous_effective_hash: string;
  activated_at: string;
  activated_by: string;
  desired_id: string | null;
  rollback_of: number | null;
  expansions: Expansion[];
}

export interface Revocation {
  seq: number;
  level: 'instance' | 'type' | 'global';
  target: string;
  revoked_at: string;
  revoked_by: string;
  reason: string;
  released_at: string | null;
  released_by: string | null;
}

export interface PolicyState {
  effective: string;
  versions: { meta: VersionMeta; body: string | null }[];
  desired: DesiredRecord[];
  revocations: Revocation[];
}

const CLAUDE_MD = [
  '# CLAUDE.md',
  '',
  '1. 一任务一 agent，一文件一持有者；写域必须在任务书里点名。',
  '2. 不 commit、不 push、不切分支；改动留在工作树，由派发方收。',
  '3. 回执一律落结构化字段：状态、改动文件、冲突、验收自评、剩余事项。',
  '4. 不可逆动作必须经主人二次认证（One-time Code）。',
  '',
].join('\n');

const GENERAL_RULES = [
  '# 通用工作规则',
  '',
  '1. 验证后才声称完成：必须先跑验证命令并确认输出。',
  '2. 共享服务不擅自重启，绝不动别人占用的资源。',
  '3. 临时产物放会话草稿目录，用完清理自己产生的。',
  '4. 真实密钥只经环境变量注入，不得写进仓库或档案。',
  '',
].join('\n');

/** 首批两条规则（ADR-026 D6），文件名就是规则名。 */
export const RULE_SUBJECTS = ['CLAUDE.md', '通用工作规则.md'];

const capabilitySet = (
  type: string,
  read: string[],
  propose: boolean,
  rules?: string[],
): string =>
  JSON.stringify({
    ai_type: type,
    // 不给 rules 就不写 rules_whitelist：该字段出现之前发布的能力集就是这样，读作零规则（D4）。
    read: rules
      ? {
          project_whitelist: read,
          deny_by_default: true,
          rules_whitelist: rules,
        }
      : { project_whitelist: read, deny_by_default: true },
    write: {
      board: true,
      board_projects: ['项目/Studiolo'],
      content_patch_only: true,
      content_direct: false,
    },
    policy: { propose_desired: propose, activate: false },
    hard_bound_skills: [],
  });

const CLAUDE_READ = [
  '项目/Studiolo',
  '项目/Semiment-Docs',
  '项目/Brief 简报站',
];
const CLAUDE_BODY = capabilitySet('claude', CLAUDE_READ, true, RULE_SUBJECTS);
const CODEX_BODY = capabilitySet('codex', ['项目/Studiolo'], false);
const PROPOSAL_BODY = capabilitySet(
  'claude',
  [...CLAUDE_READ, '项目/DocReader', '项目/NAS 网关', '项目/AI Quota Flow'],
  true,
  RULE_SUBJECTS,
);

/**
 * v3（Claude 能力集）发布后的 effective。ids.ts 只登记黑板任务冻结的几代：T-046 冻结在 v1，
 * 其余任务冻结在当前的 v3（本表第 4 版之后）；多出来的这一代由上一代推出。
 */
const AFTER_CLAUDE = sha256(`effective v3 ${POLICY_HASH.v2}`);

const firstGrant = (read: string[], propose: boolean, rules: string[] = []) => [
  ...read.map((prefix) => ({
    path: 'read.project_whitelist',
    before: null,
    after: prefix,
    note: '新类型首次获得读取范围',
  })),
  ...rules.map((rule) => ({
    path: 'read.rules_whitelist',
    before: null,
    after: rule,
    note: '新类型首次可读这条规则',
  })),
  {
    path: 'write.board_projects',
    before: null,
    after: '项目/Studiolo',
    note: '获得该项目黑板写权限',
  },
  {
    path: 'write.board',
    before: 'false',
    after: 'true',
    note: '获得黑板写权限',
  },
  {
    path: 'write.content_patch_only',
    before: 'false',
    after: 'true',
    note: '获得以 patch 形式写正文的能力',
  },
  ...(propose
    ? [
        {
          path: 'policy.propose_desired',
          before: 'false',
          after: 'true',
          note: '获得提交 desired 提案的能力（永不自动生效）',
        },
      ]
    : []),
];

export function initialPolicy(now: number): PolicyState {
  const versions = [
    {
      meta: {
        version: 1,
        kind: 'rules' as const,
        subject: RULE_SUBJECTS[0],
        retired: false,
        body_sha256: sha256(CLAUDE_MD),
        effective_hash: POLICY_HASH.v1,
        previous_effective_hash: POLICY_HASH.genesis,
        activated_at: '2026-09-08T09:14:02Z',
        activated_by: 'owner',
        desired_id: null,
        rollback_of: null,
        expansions: [],
      },
      body: CLAUDE_MD,
    },
    {
      meta: {
        version: 2,
        kind: 'rules' as const,
        subject: RULE_SUBJECTS[1],
        retired: false,
        body_sha256: sha256(GENERAL_RULES),
        effective_hash: POLICY_HASH.v2,
        previous_effective_hash: POLICY_HASH.v1,
        activated_at: '2026-09-10T03:22:15Z',
        activated_by: 'owner',
        desired_id: null,
        rollback_of: null,
        expansions: [],
      },
      body: GENERAL_RULES,
    },
    {
      meta: {
        version: 3,
        kind: 'capability_set' as const,
        subject: 'claude',
        retired: false,
        body_sha256: sha256(CLAUDE_BODY),
        effective_hash: AFTER_CLAUDE,
        previous_effective_hash: POLICY_HASH.v2,
        activated_at: '2026-09-12T02:40:31Z',
        activated_by: 'owner',
        desired_id: null,
        rollback_of: null,
        expansions: firstGrant(CLAUDE_READ, true, RULE_SUBJECTS),
      },
      body: CLAUDE_BODY,
    },
    {
      meta: {
        version: 4,
        kind: 'capability_set' as const,
        subject: 'codex',
        retired: false,
        body_sha256: sha256(CODEX_BODY),
        effective_hash: POLICY_HASH.v3,
        previous_effective_hash: AFTER_CLAUDE,
        activated_at: '2026-09-15T06:05:47Z',
        activated_by: 'owner',
        desired_id: null,
        rollback_of: null,
        expansions: firstGrant(['项目/Studiolo'], false),
      },
      body: CODEX_BODY,
    },
  ];
  const state: PolicyState = {
    effective: POLICY_HASH.v3,
    versions,
    desired: [],
    revocations: [
      {
        seq: 7,
        level: 'instance',
        target: INSTANCE.REVOKED,
        revoked_at: '2026-09-14T05:20:11Z',
        revoked_by: 'owner',
        reason: '越界写 crates/vault-broker 之外的路径，停用该实例',
        released_at: null,
        released_by: null,
      },
      {
        seq: 9,
        level: 'type',
        target: 'consumer',
        revoked_at: '2026-09-19T12:03:40Z',
        revoked_by: 'owner',
        reason: '消费端烟测结束，收回整个类型；重新发布其能力集即恢复',
        released_at: null,
        released_by: null,
      },
    ],
  };
  const submitted = iso(clockAt(now, 9.2));
  state.desired.push({
    id: DESIRED_ID,
    kind: 'capability_set',
    subject: 'claude',
    body: PROPOSAL_BODY,
    body_sha256: sha256(PROPOSAL_BODY),
    submitter_kind: 'agent',
    agent_type: 'claude',
    agent_instance: INSTANCE.C3,
    submitted_at: submitted,
    status: 'pending',
    report: validate(state, 'capability_set', 'claude', PROPOSAL_BODY).report,
    validated_at: submitted,
    activated_version: null,
  });
  return state;
}

/** 现行各项：每个 (kind, subject) 取最新一版，撤销版不列。 */
export function effectiveEntries(state: PolicyState) {
  const latest = new Map<string, { meta: VersionMeta }>();
  for (const version of state.versions)
    latest.set(`${version.meta.kind}:${version.meta.subject}`, version);
  return [...latest.values()]
    .filter(({ meta }) => !meta.retired)
    .map(({ meta }) => ({
      kind: meta.kind,
      subject: meta.subject,
      version: meta.version,
      body_sha256: meta.body_sha256,
      activated_at: meta.activated_at,
    }));
}

export function currentVersion(
  state: PolicyState,
  kind: Kind,
  subject: string,
) {
  const version = [...state.versions]
    .reverse()
    .find((row) => row.meta.kind === kind && row.meta.subject === subject);
  return version && !version.meta.retired ? version : undefined;
}

export const activeRevocations = (state: PolicyState) =>
  state.revocations.filter((row) => row.released_at === null);

export const globalRevoked = (state: PolicyState) =>
  activeRevocations(state).some((row) => row.level === 'global');

export function pendingCount(state: PolicyState) {
  return state.desired.filter((row) =>
    ['pending', 'validated'].includes(row.status),
  ).length;
}

// ---------- 校验 ----------

const token = (value: string, max: number) =>
  value.length >= 1 &&
  value.length <= max &&
  /^[a-z0-9-]+$/.test(value) &&
  !value.startsWith('-');

function subjectValid(kind: Kind, subject: string) {
  if (kind === 'rules')
    return (
      [...subject].length <= 128 &&
      subject.length > 3 &&
      subject.endsWith('.md') &&
      !subject.startsWith('.') &&
      !subject.includes('/') &&
      !subject.includes('\\')
    );
  return token(subject, kind === 'skills' ? 64 : 32);
}

const record = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);
const stringList = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every((item) => typeof item === 'string');

function exactKeys(
  value: unknown,
  required: string[],
  optional: string[] = [],
): value is Record<string, unknown> {
  return (
    record(value) &&
    required.every((key) => Object.hasOwn(value, key)) &&
    Object.keys(value).every(
      (key) => required.includes(key) || optional.includes(key),
    )
  );
}

type CapabilitySet = {
  ai_type: string;
  /** rules_whitelist 缺省即零规则（ADR-026 D3、D4）：缺字段时规范化正文里也不补。 */
  read: {
    project_whitelist: string[];
    deny_by_default: boolean;
    rules_whitelist?: string[];
  };
  write: {
    board: boolean;
    board_projects: string[];
    content_patch_only: boolean;
    content_direct: boolean;
  };
  policy: { propose_desired: boolean; activate: boolean };
  hard_bound_skills: { when: string; must_use: string }[];
};

function parseCapabilitySet(body: string): CapabilitySet | null {
  let value: unknown;
  try {
    value = JSON.parse(body);
  } catch {
    return null;
  }
  if (
    !exactKeys(
      value,
      ['ai_type', 'read', 'write', 'policy'],
      ['hard_bound_skills'],
    ) ||
    typeof value.ai_type !== 'string' ||
    !exactKeys(
      value.read,
      ['project_whitelist', 'deny_by_default'],
      ['rules_whitelist'],
    ) ||
    !exactKeys(
      value.write,
      ['board', 'content_patch_only', 'content_direct'],
      ['board_projects'],
    ) ||
    !exactKeys(value.policy, ['propose_desired', 'activate'])
  )
    return null;
  const read = value.read as Record<string, unknown>;
  const write = value.write as Record<string, unknown>;
  const policy = value.policy as Record<string, unknown>;
  const bindings = value.hard_bound_skills ?? [];
  const rules = Object.hasOwn(read, 'rules_whitelist')
    ? read.rules_whitelist
    : undefined;
  if (
    !stringList(read.project_whitelist) ||
    typeof read.deny_by_default !== 'boolean' ||
    !(rules === undefined || stringList(rules)) ||
    typeof write.board !== 'boolean' ||
    !stringList(write.board_projects ?? []) ||
    typeof write.content_patch_only !== 'boolean' ||
    typeof write.content_direct !== 'boolean' ||
    typeof policy.propose_desired !== 'boolean' ||
    typeof policy.activate !== 'boolean' ||
    !Array.isArray(bindings) ||
    !bindings.every(
      (item) =>
        exactKeys(item, ['when', 'must_use']) &&
        typeof item.when === 'string' &&
        typeof item.must_use === 'string',
    )
  )
    return null;
  return {
    ai_type: value.ai_type,
    read: {
      project_whitelist: read.project_whitelist,
      deny_by_default: read.deny_by_default,
      ...(rules === undefined ? {} : { rules_whitelist: rules }),
    },
    write: {
      board: write.board,
      board_projects: (write.board_projects ?? []) as string[],
      content_patch_only: write.content_patch_only,
      content_direct: write.content_direct,
    },
    policy: {
      propose_desired: policy.propose_desired,
      activate: policy.activate,
    },
    hard_bound_skills: bindings as { when: string; must_use: string }[],
  };
}

function flatten(prefix: string, value: unknown, out: Map<string, string>) {
  if (Array.isArray(value)) {
    if (!value.length) out.set(prefix, '[]');
    value.forEach((item, index) => flatten(`${prefix}[${index}]`, item, out));
  } else if (record(value)) {
    const keys = Object.keys(value).sort();
    if (!keys.length) out.set(prefix, '{}');
    for (const key of keys)
      flatten(prefix ? `${prefix}.${key}` : key, value[key], out);
  } else out.set(prefix, JSON.stringify(value));
}

function jsonDiff(before: unknown, after: unknown): Report['diff'] {
  const left = new Map<string, string>();
  const right = new Map<string, string>();
  if (before !== undefined) flatten('', before, left);
  if (after !== undefined) flatten('', after, right);
  const entries: DiffEntry[] = [];
  for (const [path, value] of [...left].sort(([a], [b]) => (a < b ? -1 : 1))) {
    const next = right.get(path);
    if (next === undefined)
      entries.push({ op: 'removed', path, before: value, after: null });
    else if (next !== value)
      entries.push({ op: 'changed', path, before: value, after: next });
  }
  for (const [path, value] of [...right].sort(([a], [b]) => (a < b ? -1 : 1)))
    if (!left.has(path))
      entries.push({ op: 'added', path, before: null, after: value });
  return summarize('json', entries);
}

function textDiff(before: string, after: string): Report['diff'] {
  const old = before ? before.split('\n') : [];
  const next = after ? after.split('\n') : [];
  const table = Array.from({ length: old.length + 1 }, () =>
    new Array<number>(next.length + 1).fill(0),
  );
  for (let i = old.length - 1; i >= 0; i -= 1)
    for (let j = next.length - 1; j >= 0; j -= 1)
      table[i][j] =
        old[i] === next[j]
          ? table[i + 1][j + 1] + 1
          : Math.max(table[i + 1][j], table[i][j + 1]);
  const entries: DiffEntry[] = [];
  let i = 0;
  let j = 0;
  while (i < old.length || j < next.length) {
    if (i < old.length && j < next.length && old[i] === next[j]) {
      i += 1;
      j += 1;
    } else if (
      j < next.length &&
      (i === old.length || table[i][j + 1] >= table[i + 1][j])
    ) {
      entries.push({
        op: 'added',
        path: `new:${j + 1}`,
        before: null,
        after: next[j],
      });
      j += 1;
    } else {
      entries.push({
        op: 'removed',
        path: `old:${i + 1}`,
        before: old[i],
        after: null,
      });
      i += 1;
    }
  }
  return summarize('text', entries);
}

function summarize(format: string, entries: DiffEntry[]): Report['diff'] {
  return {
    format,
    entries: entries.slice(0, 200),
    added: entries.filter((entry) => entry.op === 'added').length,
    removed: entries.filter((entry) => entry.op === 'removed').length,
    changed: entries.filter((entry) => entry.op === 'changed').length,
    truncated: entries.length > 200,
  };
}

const covered = (prefix: string, set: string[]) =>
  set.some((item) => item === prefix || prefix.startsWith(`${item}/`));
const CONSTRAINTS = ['禁止', '绝不', '不得', '一律', '必须', '永不', '必叠'];

/** 按当前 effective 校验一份 desired；返回报告与规范化正文（失败时为 null）。 */
export function validate(
  state: PolicyState,
  kind: Kind,
  subject: string,
  body: string | null,
): { report: Report; canonical: string | null } {
  const current = currentVersion(state, kind, subject);
  const report: Report = {
    ok: false,
    kind,
    subject,
    retire: body === null,
    problems: [],
    diff: summarize(kind === 'rules' ? 'text' : 'json', []),
    expansions: [],
    narrowings: [],
    base_effective_hash: state.effective,
    current_version: current?.meta.version ?? null,
    body_sha256: sha256(''),
  };
  if (!subjectValid(kind, subject)) {
    report.problems.push({
      path: 'subject',
      message: 'subject 不满足该类别的命名规则',
    });
    return { report, canonical: null };
  }
  if (body === null) {
    if (!current)
      report.problems.push({
        path: 'subject',
        message: '该 subject 没有生效版本，无可撤销',
      });
    else {
      report.diff =
        kind === 'rules'
          ? textDiff(current.body ?? '', '')
          : jsonDiff(JSON.parse(current.body ?? 'null'), undefined);
      report.expansions.push({
        path: '(retire)',
        before: `version ${current.meta.version}`,
        after: null,
        note:
          kind === 'rules'
            ? '撤销整份规则母本：全部约束消失，需人工确认'
            : kind === 'skills'
              ? '撤销 skill：索引候选消失'
              : '撤销能力集：该类型回到零权限（安全方向）',
      });
    }
    report.ok = report.problems.length === 0;
    return { report, canonical: null };
  }
  if (body.includes('\0') || new TextEncoder().encode(body).length > 262_144) {
    report.problems.push({ path: 'body', message: '正文超过上限或含 NUL' });
    return { report, canonical: null };
  }
  let canonical: string;
  if (kind === 'rules') {
    canonical = body.replace(/\r\n/g, '\n');
    if (!canonical.trim()) {
      report.problems.push({ path: 'body', message: '规则母本不能为空' });
      return { report, canonical: null };
    }
    report.diff = textDiff(current?.body ?? '', canonical);
    for (const entry of report.diff.entries)
      if (
        entry.op === 'removed' &&
        CONSTRAINTS.some((marker) => entry.before?.includes(marker))
      )
        report.expansions.push({
          path: entry.path,
          before: entry.before,
          after: null,
          note: '删除了约束性语句，可能放宽（候选，人工判断）',
        });
  } else if (kind === 'skills') {
    let skill: unknown;
    try {
      skill = JSON.parse(body);
    } catch {
      skill = undefined;
    }
    if (
      !exactKeys(skill, ['name', 'summary', 'markdown'], ['triggers']) ||
      typeof skill.name !== 'string' ||
      typeof skill.summary !== 'string' ||
      typeof skill.markdown !== 'string' ||
      !stringList(skill.triggers ?? [])
    ) {
      report.problems.push({
        path: 'line 1:1',
        message: 'JSON 不符合形状：字段缺失、多余或类型不符',
      });
      return { report, canonical: null };
    }
    const normalized = {
      name: skill.name,
      summary: skill.summary,
      triggers: (skill.triggers ?? []) as string[],
      markdown: skill.markdown,
    };
    if (normalized.name !== subject)
      report.problems.push({
        path: 'name',
        message: 'skill 名必须等于 subject',
      });
    if (!normalized.summary.trim() || [...normalized.summary].length > 200)
      report.problems.push({
        path: 'summary',
        message: '摘要须为 1..=200 字符',
      });
    if (normalized.triggers.length > 16)
      report.problems.push({ path: 'triggers', message: '触发条件最多 16 条' });
    if (!normalized.markdown.trim())
      report.problems.push({ path: 'markdown', message: 'skill 正文不能为空' });
    canonical = JSON.stringify(normalized);
    report.diff = jsonDiff(
      current?.body ? JSON.parse(current.body) : undefined,
      normalized,
    );
  } else {
    const set = parseCapabilitySet(body);
    if (!set) {
      report.problems.push({
        path: 'line 1:1',
        message: 'JSON 不符合形状：字段缺失、多余或类型不符',
      });
      return { report, canonical: null };
    }
    if (set.ai_type !== subject)
      report.problems.push({
        path: 'ai_type',
        message: 'ai_type 必须等于 subject',
      });
    canonical = JSON.stringify(set);
    capabilityChecks(report, set, current?.body ?? null);
  }
  report.body_sha256 = sha256(canonical);
  report.ok = report.problems.length === 0;
  return { report, canonical };
}

function capabilityChecks(
  report: Report,
  set: CapabilitySet,
  previousBody: string | null,
) {
  const problem = (path: string, message: string) =>
    report.problems.push({ path, message });
  if (!set.read.deny_by_default)
    problem('read.deny_by_default', '必须为 true：白名单之外不进入计算');
  if (set.write.content_direct)
    problem('write.content_direct', '永久为 false：AI 不整页覆盖人写正文');
  if (set.write.board && !set.write.board_projects.length)
    problem('write.board', '黑板写入必须有非空显式 write.board_projects');
  if (!set.write.board && set.write.board_projects.length)
    problem('write.board_projects', '黑板只读时写项目必须为空');
  for (const prefix of set.write.board_projects)
    if (
      !prefix.startsWith('项目/') ||
      prefix.split('/').length !== 2 ||
      !covered(prefix, set.read.project_whitelist)
    )
      problem(
        'write.board_projects',
        '必须为唯一、合法且位于读取白名单内的完整项目路径',
      );
  if (set.policy.activate)
    problem('policy.activate', '永久为 false：AI 无 One-time Code 路径');
  set.read.project_whitelist.forEach((prefix, index) => {
    if (
      !prefix ||
      prefix.split('/').some((part) => !part || part === '.' || part === '..')
    )
      problem(`read.project_whitelist[${index}]`, '不是合法的逻辑路径前缀');
    else if (set.read.project_whitelist.indexOf(prefix) !== index)
      problem(`read.project_whitelist[${index}]`, '重复的白名单前缀');
  });
  // 规则读名单：元素须是合法的规则名、不得重复（ADR-026 D3）；缺字段即零规则。
  const rules = set.read.rules_whitelist ?? [];
  rules.forEach((rule, index) => {
    if (!subjectValid('rules', rule))
      problem(`read.rules_whitelist[${index}]`, '不是合法的规则名');
    else if (rules.indexOf(rule) !== index)
      problem(`read.rules_whitelist[${index}]`, '重复的规则名');
  });
  set.hard_bound_skills.forEach((_, index) =>
    problem(`hard_bound_skills[${index}].must_use`, '指向的 skill 尚未发布'),
  );
  const previous = previousBody ? parseCapabilitySet(previousBody) : null;
  report.diff = jsonDiff(
    previousBody ? JSON.parse(previousBody) : undefined,
    JSON.parse(JSON.stringify(set)),
  );
  const oldRead = previous?.read.project_whitelist ?? [];
  for (const prefix of set.read.project_whitelist)
    if (!covered(prefix, oldRead))
      report.expansions.push({
        path: 'read.project_whitelist',
        before: null,
        after: prefix,
        note: previous ? '读取范围变宽' : '新类型首次获得读取范围',
      });
  for (const prefix of oldRead)
    if (!covered(prefix, set.read.project_whitelist))
      report.narrowings.push({
        path: 'read.project_whitelist',
        before: prefix,
        after: null,
        note: '读取范围收窄',
      });
  const oldRules = previous?.read.rules_whitelist ?? [];
  for (const rule of rules)
    if (!oldRules.includes(rule))
      report.expansions.push({
        path: 'read.rules_whitelist',
        before: null,
        after: rule,
        note: previous ? '可读的规则增加' : '新类型首次可读这条规则',
      });
  for (const rule of oldRules)
    if (!rules.includes(rule))
      report.narrowings.push({
        path: 'read.rules_whitelist',
        before: rule,
        after: null,
        note: '可读的规则减少',
      });
  const oldBoard = previous?.write.board_projects ?? [];
  for (const prefix of set.write.board_projects)
    if (!oldBoard.includes(prefix))
      report.expansions.push({
        path: 'write.board_projects',
        before: null,
        after: prefix,
        note: '获得该项目黑板写权限',
      });
  for (const prefix of oldBoard)
    if (!set.write.board_projects.includes(prefix))
      report.narrowings.push({
        path: 'write.board_projects',
        before: prefix,
        after: null,
        note: '收回该项目黑板写权限',
      });
  const flags: [string, boolean, boolean, string][] = [
    ['write.board', !!previous?.write.board, set.write.board, '获得黑板写权限'],
    [
      'write.content_patch_only',
      !!previous?.write.content_patch_only,
      set.write.content_patch_only,
      '获得以 patch 形式写正文的能力',
    ],
    [
      'policy.propose_desired',
      !!previous?.policy.propose_desired,
      set.policy.propose_desired,
      '获得提交 desired 提案的能力（永不自动生效）',
    ],
  ];
  for (const [path, before, after, note] of flags) {
    if (!before && after)
      report.expansions.push({ path, before: 'false', after: 'true', note });
    else if (before && !after)
      report.narrowings.push({
        path,
        before: 'true',
        after: 'false',
        note: '能力收回',
      });
  }
}

/** 发布一版（desired 或回滚），返回 Activation；调用方负责 L3 与 effective 哈希核对。 */
export function publish(
  state: PolicyState,
  input: {
    kind: Kind;
    subject: string;
    body: string | null;
    report: Report;
    desiredId: string | null;
    rollbackOf: number | null;
  },
  now: number,
) {
  const version = state.versions.length + 1;
  const previous = state.effective;
  const effective = sha256(`effective v${version} ${previous}`);
  const clearedType =
    input.kind === 'capability_set' &&
    input.body !== null &&
    state.revocations.some(
      (row) =>
        row.released_at === null &&
        row.level === 'type' &&
        row.target === input.subject,
    );
  if (clearedType)
    for (const row of state.revocations)
      if (
        row.released_at === null &&
        row.level === 'type' &&
        row.target === input.subject
      ) {
        row.released_at = iso(now);
        row.released_by = 'owner';
      }
  const meta: VersionMeta = {
    version,
    kind: input.kind,
    subject: input.subject,
    retired: input.body === null,
    body_sha256: input.body === null ? sha256('') : sha256(input.body),
    effective_hash: effective,
    previous_effective_hash: previous,
    activated_at: iso(now),
    activated_by: 'owner',
    desired_id: input.desiredId,
    rollback_of: input.rollbackOf,
    expansions: input.report.expansions,
  };
  state.versions.push({ meta, body: input.body });
  state.effective = effective;
  return {
    version,
    kind: meta.kind,
    subject: meta.subject,
    retired: meta.retired,
    body_sha256: meta.body_sha256,
    effective_hash: effective,
    previous_effective_hash: previous,
    expansions: meta.expansions,
    desired_id: meta.desired_id,
    rollback_of: meta.rollback_of,
    cleared_type_revocation: clearedType,
    activated_by: 'owner',
  };
}
