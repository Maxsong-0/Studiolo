import { useEffect, useState } from 'react';
import { agoLabel, agoValidFor } from './ago';

/** 到点后多等这一点再算：有的浏览器把时钟取整，卡着点算会原地空转几次。 */
const SLACK_MS = 50;

const pad = (value: number) => String(value).padStart(2, '0');

function exact(at: number): string {
  const date = new Date(at);
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(
    date.getSeconds(),
  )}`;
}

/**
 * 「N 秒前」：自己走字（T-196）。只在说法该变的那一刻重画自己，不靠上层重画；
 * 页面切回来时立刻对一次表（后台标签页的定时器会被浏览器拖慢）。
 * 不是 live region：每跳一次都念一遍只会吵人。
 */
export function Ago({ at }: { at: number }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    let timer: number | undefined;
    const arm = () => {
      timer = window.setTimeout(tick, agoValidFor(Date.now() - at) + SLACK_MS);
    };
    const tick = () => {
      setNow(Date.now());
      arm();
    };
    const resync = () => {
      if (document.visibilityState !== 'visible') return;
      window.clearTimeout(timer);
      tick();
    };
    arm();
    document.addEventListener('visibilitychange', resync);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener('visibilitychange', resync);
    };
  }, [at]);
  // at 刚换成新的一刻时 now 还是上一次走字的时刻，差是负数，agoLabel 按 0 算，写「刚刚」。
  return (
    <time dateTime={new Date(at).toISOString()} title={exact(at)}>
      {agoLabel(now - at)}
    </time>
  );
}
