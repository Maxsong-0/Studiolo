import { render } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { FileOperationsAccess } from '../../components/FileOperationsAccess';
import type { FileOperationRecord } from '../../fileOperationModel';
import { canonical } from '../canonicalFixtures';

const state = vi.hoisted(() => ({
  phase: 'ready',
  record: null as FileOperationRecord | null,
}));
vi.mock('../../useFileOperations', () => ({
  useFileOperations: () => ({
    phase: state.phase,
    vault: null,
    retry: vi.fn(),
  }),
}));
vi.mock('../../fileOperationContext', () => ({
  useFileOperationStore: () => ({ record: state.record }),
}));

it('发送中和结果不确定会向顶栏报告待核对；确定结果解除标记；入口隐藏时报告 null', () => {
  const onEntryChange = vi.fn();
  const element = () => (
    <FileOperationsAccess onChanged={vi.fn()} onEntryChange={onEntryChange} />
  );
  const view = render(element());
  expect(onEntryChange).toHaveBeenLastCalledWith(
    expect.objectContaining({ deletable: true, pending: false }),
  );
  for (const phase of [
    'sending',
    'uncertain',
    'committed',
    'rejected',
  ] as const) {
    state.record = {
      phase,
      message: '测试回执',
      request: {
        kind: 'delete',
        vault_uuid: canonical.vault_uuid,
        path: canonical.path,
        expected_revision: canonical.revision,
        idempotency_key: 'fixture-file-key',
      },
    };
    view.rerender(element());
    expect(onEntryChange).toHaveBeenLastCalledWith(
      expect.objectContaining({
        pending: phase === 'sending' || phase === 'uncertain',
      }),
    );
  }
  state.phase = 'disabled';
  state.record = null;
  view.rerender(element());
  expect(onEntryChange).toHaveBeenLastCalledWith(null);
});
