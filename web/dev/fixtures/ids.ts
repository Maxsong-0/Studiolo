/**
 * 假数据里的全部标识。ULID 与 SHA-256 都是离线算好的固定值（按创建时刻与说明文字生成），
 * 让黑板、执行、回放、策略几个接口之间能互相引用同一个任务、实例与哈希。
 *
 * 只在 `vite serve` 的假数据模式里使用；生产构建不引用本目录。
 */
import { createHash } from 'node:crypto';

/** 只在假数据里出现的标记：生产构建产物里 grep 它必须无结果。 */
export const FIXTURE_MARKER = 'exec-0922';

export const sha256 = (text: string): string =>
  createHash('sha256').update(text).digest('hex');

/** 黑板任务 ID（ULID）。键名是 vault 里的任务编号。 */
export const TASK = {
  T046: '01M24Z4SY0KMSET797WZCWPABE',
  T092: '01M2M0VCH85C0083DRCXWFHKH3',
  T093: '01M2M10AQR1C19VAZ64RWBZGQA',
  T106: '01M31K73HR69MZ4WSNE5ZP1KQR',
  T108: '01M31KDVC0P86Z59RRKB25DJMP',
  T109: '01M339BVSG53GPPAS91G0ZT3P0',
  T130: '01M34DG8NRY6FQAQ3GZT6PWSFW',
  B021: '01M2YCPA6GS8NGH5XYFX6DBV19',
} as const;
export type TaskKey = keyof typeof TASK;

/** 每个任务最近一次 attempt 的 ID（`A-` 前缀 + ULID）。 */
export const ATTEMPT: Record<TaskKey, string> = {
  T046: 'A-01M2S2Q9C01SZDV4TS7PW203RH',
  T092: 'A-01M33V67R0EZFEJW3HP3ZGDBAT',
  T093: 'A-01M3389ZT0YBGSSM3HCAQ0CMSW',
  T106: 'A-01M33Q2D00W83YDTTNTM6GRMV0',
  T108: 'A-01M33QXVX077WN8MWM7BNNCGHT',
  T109: 'A-01M33A0XM0S5DJ1TFWKS3GS9DW',
  T130: 'A-01M34E4A907ZQ46Z7J3E1D35VS',
  B021: 'A-01M33EFQY0MEYXA2XZFDT203P7',
};

/**
 * 接入实例。R* 是一条「执行线」的根实例（负责创建任务），C* 是它派生出来认领任务的子实例。
 * 画布上的五条线：重设计线 R1、M4 接续线 R2、全项目收尾线 R3、接入线 R4、Codex 根统筹 R5。
 */
export const INSTANCE = {
  R1: '01M34CMPV0BEJ8K1EYA51BYN9N',
  C1: '01M34E0N302Z162ECGQMN36RG6',
  R2: '01M2J18Y00C41N2JMJC4V0HA0R',
  C2a: '01M2M1BQ0063DC947274TPMAAJ',
  C2b: '01M2M1DHK0144GXGZBP56N8K48',
  R3: '01M2Y57FM0P5XWGV9MV79YTJ1J',
  C3: '01M339QRN0QBBC7CSN8RPC2EZ7',
  C3b: '01M33EA850K5T0YMN492BEF54S',
  R4: '01M31JGSP0AWNDMM7A9YQ3574J',
  C4a: '01M33PWX700D4VC0E68BG36RP2',
  C4b: '01M33QW1A0DX6M32A2ZXBGM4Q8',
  R5: '01M230M2G06MRPSZPY22W5TYNY',
  /** 已被实例级吊销的旧 Codex 实例，只出现在策略页。 */
  REVOKED: '01M2EXPWW0EK4N0KNMAAV8V0T3',
} as const;
export type InstanceKey = keyof typeof INSTANCE;

/** 每个实例的类型与父实例；子实例的链从根开始。 */
export const INSTANCE_TREE: Record<
  InstanceKey,
  { type: 'claude' | 'codex'; parent: InstanceKey | null }
> = {
  R1: { type: 'claude', parent: null },
  C1: { type: 'claude', parent: 'R1' },
  R2: { type: 'codex', parent: null },
  C2a: { type: 'codex', parent: 'R2' },
  C2b: { type: 'codex', parent: 'R2' },
  R3: { type: 'claude', parent: null },
  C3: { type: 'claude', parent: 'R3' },
  C3b: { type: 'claude', parent: 'R3' },
  R4: { type: 'claude', parent: null },
  C4a: { type: 'claude', parent: 'R4' },
  C4b: { type: 'claude', parent: 'R4' },
  R5: { type: 'codex', parent: null },
  REVOKED: { type: 'codex', parent: null },
};

/** 从根到该实例的链（含自身）。 */
export function lineage(key: InstanceKey): InstanceKey[] {
  const chain: InstanceKey[] = [];
  for (
    let current: InstanceKey | null = key;
    current;
    current = INSTANCE_TREE[current].parent
  )
    chain.unshift(current);
  return chain;
}

/**
 * 策略平面的 effective 哈希：genesis → v1 → v2 → v3（当前）。
 * T-046 创建于 v1 生效期间，冻结在 v1 上，因此显示「按旧版规则执行」。
 */
export const POLICY_HASH = {
  genesis: '4f4d8d327d4b87a585620bdf4a46ae4167da53369bdd3533bf68ddaa0dccd215',
  v1: '976b2c6684e5469513e01fc98982166832177fc9af3b3d3006c99e3b92041f42',
  v2: 'f5732303ca91357442f41f5628f6a1b449cb163791625aa64206d26321ecea9d',
  v3: '17a0d5031f4f6056b207922f455036c7bc19cf78f8d129214d981e0399e59e5b',
} as const;

/** 待复核的 AI 提案（desired）。 */
export const DESIRED_ID = '01M33ZA2G0HQ8K8HMTV4NH3KKP';

export const VAULT_UUID = '6f1c2e4a-9b3d-4c57-8e21-5a7d0c9f3b64';
export const OWNER_SCOPE =
  '4d82e815378799cc4fd04d6c60d1de530343f539412ebd45a0e7d17180edf4f5';
export const SESSION_ID = 'v2-22a85f98946dc94f';
export const OTHER_SESSION_ID = 'v2-533c4c2946d5f879';
export const CSRF_TOKEN =
  'c2.22a85f98946dc94f.39bb18dce2973017cfe81b79422d873ea47bb63d29da138ab00418b5ad6bed84';
/** 恢复码批次号：16 位十六进制，首位 0–7。 */
export const RECOVERY_BATCH = '37ac0a48e6b0a81e';
