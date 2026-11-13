import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import { createAttention } from './attention';
import {
  createAutoRefresh,
  NO_AUTO_REFRESH,
  type AutoRefresh,
  type AutoRefreshClock,
  type AutoRefreshStatus,
  type Revalidate,
} from './autoRefresh';

const browserClock: AutoRefreshClock = {
  now: () => Date.now(),
  setTimer: (run, ms) => window.setTimeout(run, ms),
  clearTimer: (handle) => window.clearTimeout(handle as number),
};

/**
 * 现在读会不会打扰人：任何原生弹层开着都算（阅读器、⌘K、草稿、验证码、最近删除、会话）。
 * 阅读器和 ⌘K 面板按快照代次取键，底下换了快照它们会整个重挂，滚动位置和输入的词都丢。
 */
const dialogOpen = () => document.querySelector('dialog[open]') !== null;

/** 算「有人在操作」的事件；都只记一个时刻，挂成被动监听。 */
const INTERACTIONS = [
  'pointerdown',
  'pointermove',
  'keydown',
  'wheel',
  'touchstart',
] as const;

function subscribeVisibility(listener: () => void) {
  document.addEventListener('visibilitychange', listener);
  return () => document.removeEventListener('visibilitychange', listener);
}
const pageVisible = () => document.visibilityState === 'visible';

/** 全站那一个自动刷新调度器；不在 App 里（单独渲染的视图、测试）时是不读任何东西的那一个。 */
export const AutoRefreshContext = createContext<AutoRefresh>(NO_AUTO_REFRESH);

/**
 * 给 App 用：建全站唯一的自动刷新调度器，并决定它什么时候该读（T-196）。四样都满足才读：
 *
 * - `active`：当前视图要自动刷新（驾驶舱、协作拓扑），而且没有开着会被打断的东西；
 * - 没有手动读取在途（`reading`）：手动读取读完时从那一刻重新计时，不紧跟着再读一遍；
 * - 页面看得见：切走不读，切回来数据过期了就立刻补读；
 * - 页面上有人在操作（attention.ts）：一段时间没有操作就停，免得把服务端的闲置登出一直往后推。
 */
export function useAutoRefresh({
  active,
  reading,
  failed,
}: {
  active: boolean;
  /** 手动读取（首次读取、刷新）在途。 */
  reading: boolean;
  /** 最近一次手动读取没读成。 */
  failed: boolean;
}): AutoRefresh {
  const [auto] = useState(() =>
    createAutoRefresh(browserClock, { blocked: dialogOpen }),
  );
  const [attended, setAttended] = useState(true);
  const visible = useSyncExternalStore(
    subscribeVisibility,
    pageVisible,
    () => true,
  );

  // 写在「该不该读」那个 effect 前面：同一次提交里先记下刚读完，再启用，启用时就不会立刻又读一遍。
  const wasReading = useRef(reading);
  useEffect(() => {
    if (wasReading.current && !reading) auto.noteManualRead(!failed);
    wasReading.current = reading;
  }, [auto, reading, failed]);

  useEffect(() => {
    const attention = createAttention(browserClock, setAttended);
    const touch = () => attention.touch();
    const returned = () => {
      if (pageVisible()) attention.touch();
    };
    for (const type of INTERACTIONS)
      window.addEventListener(type, touch, { capture: true, passive: true });
    window.addEventListener('focus', touch);
    document.addEventListener('visibilitychange', returned);
    attention.start();
    return () => {
      for (const type of INTERACTIONS)
        window.removeEventListener(type, touch, { capture: true });
      window.removeEventListener('focus', touch);
      document.removeEventListener('visibilitychange', returned);
      attention.stop();
    };
  }, []);

  const wanted = active && !reading && visible;
  useEffect(() => {
    auto.setEnabled(wanted && attended, wanted && !attended);
  }, [auto, wanted, attended]);
  useEffect(() => () => auto.setEnabled(false), [auto]);

  return auto;
}

/** 把一路读取登记给调度器；`revalidate` 的引用要稳定（useCallback），否则每次重画都重新登记。 */
export function useAutoRefreshReader(
  auto: AutoRefresh,
  revalidate: Revalidate,
  order: number,
) {
  useEffect(() => auto.register(revalidate, order), [auto, revalidate, order]);
}

/** 调度器现在的状态；跟着它变。 */
export function useAutoRefreshStatus(): AutoRefreshStatus & {
  runNow: () => void;
} {
  const auto = useContext(AutoRefreshContext);
  const status = useSyncExternalStore(auto.subscribe, auto.status);
  return { ...status, runNow: auto.runNow };
}
