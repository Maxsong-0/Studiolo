import { useEffect, useState } from 'react';
import { errorMessage } from '../api';
import { boardOverviewDisabled, fetchBoardOverview } from '../boardOverviewApi';
import { BOARD_OVERVIEW_DEFAULT_WINDOW_HOURS } from '../boardOverviewModel';
import type { BoardOverview } from '../boardOverviewModel';

export type PaletteOverview =
  | { kind: 'loading' }
  | { kind: 'ready'; overview: BoardOverview }
  /** 本机没装配黑板（404）：没有黑板任务可跳，不算错误。 */
  | { kind: 'disabled' }
  | { kind: 'error'; message: string };

/**
 * 面板打开时读一次跨项目黑板汇总（与驾驶舱、协作拓扑同一个接口、同一个默认时间窗），
 * 供「跳到黑板任务」使用。关面板即取消；读不到只影响黑板任务这几行，其余命令照常。
 */
export function usePaletteOverview(): PaletteOverview {
  const [state, setState] = useState<PaletteOverview>({ kind: 'loading' });
  useEffect(() => {
    const controller = new AbortController();
    fetchBoardOverview(
      BOARD_OVERVIEW_DEFAULT_WINDOW_HOURS,
      controller.signal,
    ).then(
      (overview) => {
        if (!controller.signal.aborted) setState({ kind: 'ready', overview });
      },
      (error: unknown) => {
        if (controller.signal.aborted) return;
        setState(
          boardOverviewDisabled(error)
            ? { kind: 'disabled' }
            : { kind: 'error', message: errorMessage(error) },
        );
      },
    );
    return () => controller.abort();
  }, []);
  return state;
}
