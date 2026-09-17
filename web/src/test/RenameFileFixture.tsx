import { useSyncExternalStore } from 'react';
import type { EditorController } from '../EditorController';
import { RenameFile } from '../components/RenameFile';

export function RenameFileFixture({
  controller,
}: {
  controller: EditorController;
}) {
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  return (
    <>
      <textarea
        aria-label="测试正文"
        value={state.draft}
        onChange={(event) => controller.change(event.target.value)}
      />
      <RenameFile controller={controller} state={state} />
    </>
  );
}
