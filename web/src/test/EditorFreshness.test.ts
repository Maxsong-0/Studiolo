import { afterEach, expect, it, vi } from 'vitest';
import { EditorController } from '../EditorController';
import { DraftStore } from '../draftStore';
import { installCanonicalWriter } from '../canonicalApi';
import type { Replacement } from '../canonicalModel';
import { canonical, committed, ownerScope } from './canonicalFixtures';
import { jsonResponse } from './fixtures';

afterEach(() => vi.useRealTimers());
it.each([false, true])(
  '保存后200陈旧有界重查；持续陈旧=%s不重放POST',
  async (alwaysStale) => {
    vi.useFakeTimers();
    const fresh = {
      ...canonical,
      revision: 'b'.repeat(64),
      content: '本次保存',
    };
    const stale = {
      ...fresh,
      editing: { allowed: false, reason: 'stale', max_bytes: 262144 },
      freshness: {
        ...canonical.freshness!,
        stale: true,
        indexed_through: '1',
        committed_through: '2',
        reason: 'index_lag',
      },
    };
    const read = vi
      .fn()
      .mockResolvedValueOnce(jsonResponse(canonical))
      .mockImplementation(async () =>
        jsonResponse(alwaysStale ? stale : fresh),
      );
    read.mockResolvedValueOnce(jsonResponse(stale));
    vi.stubGlobal('fetch', read);
    const writer = vi.fn(async (request: Replacement) => committed(request));
    const uninstall = installCanonicalWriter(writer);
    const editor = new EditorController(
      canonical.path,
      new DraftStore().forScope(ownerScope),
    );
    try {
      await editor.load();
      editor.change(fresh.content);
      const saving = editor.save();
      await vi.advanceTimersByTimeAsync(149);
      expect(editor.getSnapshot().phase).toBe('verifying');
      await vi.advanceTimersByTimeAsync(alwaysStale ? 451 : 1);
      await saving;
      expect(writer).toHaveBeenCalledOnce();
      expect(read).toHaveBeenCalledTimes(alwaysStale ? 4 : 3);
      expect(editor.getSnapshot().phase).toBe(
        alwaysStale ? 'blocked' : 'clean',
      );
      if (alwaysStale)
        expect(editor.getSnapshot().message).toContain('完整视图尚未追平');
      await vi.advanceTimersByTimeAsync(5000);
      expect(writer).toHaveBeenCalledOnce();
    } finally {
      editor.stop();
      uninstall();
    }
  },
);
