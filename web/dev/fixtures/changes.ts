/** 合成内容变化；仅开发模式，不接触真实文件或审计。 */
import type { ChangesPage, ChangeEntry } from '../../src/changesModel.ts';
import { VAULT_UUID, TASK, ATTEMPT } from './ids.ts';
const EPOCH = 'd11d3d3a-2943-4cae-bf20-000000000001';
export function changesFixture(
  now: number,
  sequence: number,
  project: string | null,
  snapshot: string,
  seen: boolean,
): ChangesPage {
  const specs = [
    ['项目/Studiolo/00-总览/INDEX.md', 'agent'],
    ['项目/Semiment/00-总览/INDEX.md', 'obsidian'],
    ['项目/Brief 简报站/00-总览/INDEX.md', 'human'],
    ['项目/Studiolo/04-协调/TASKS.md', 'external'],
  ] as const;
  const entries: ChangeEntry[] = specs.map(([path, kind], i) => ({
    id:
      kind === 'external'
        ? `o:${EPOCH}:2`
        : `b:${sequence - i}:${'a'.repeat(60)}${String(i).padStart(4, '0')}`,
    source: kind === 'external' ? 'observation' : 'broker',
    kind: 'replaced',
    path,
    sequence: kind === 'external' ? null : String(sequence - i),
    generation: kind === 'external' ? '243' : null,
    observation: kind === 'external' ? '2' : null,
    at_ms: String(now - (i + 1) * 90000),
    actor:
      kind === 'agent'
        ? {
            kind: 'agent',
            agent_type: 'claude',
            instance_id: 'fixture-instance',
            task_id: TASK.T046,
            attempt_id: ATTEMPT.T046,
          }
        : kind === 'human'
          ? {
              kind: 'human',
              issuer: 'https://fixture.invalid',
              subject: '虚构主人',
            }
          : kind === 'obsidian'
            ? { kind: 'obsidian' }
            : null,
    previous_revision: 'a'.repeat(64),
    revision: 'b'.repeat(64),
    bytes: String(128 + i),
    counterpart: null,
    gap: null,
    unread: !seen,
  }));
  entries.push({
    id: `o:${EPOCH}:1`,
    source: 'observation',
    kind: 'gap',
    path: null,
    sequence: null,
    generation: '1',
    observation: '1',
    at_ms: String(now - 3600000),
    actor: null,
    previous_revision: null,
    revision: null,
    bytes: null,
    counterpart: null,
    gap: 'startup',
    unread: !seen,
  });
  return {
    vault_uuid: VAULT_UUID,
    project,
    entries: entries.filter(
      (r) =>
        r.path === null || project === null || r.path.startsWith(project + '/'),
    ),
    next_cursor: null,
    snapshot,
    through: {
      sequence: String(sequence),
      epoch: EPOCH,
      observation: '2',
      generation: '243',
    },
    history_incomplete: false,
    retired: false,
    seen_complete: true,
  };
}
