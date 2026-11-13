import { useEffect, useState } from 'react';
import { remainingLabel, remainingMs, type AuthWindow } from './recentAuth';
import '../styles/policy.css';

/**
 * 「二次认证还有效 m 分 s 秒」那一行（T-200）：每秒走一次字，到点就不画了并告诉上面。
 * 走字的状态关在这个小组件里，外面的页面不跟着每秒重画。
 * 不是 live region：每秒都在变的字不该让读屏每秒念一遍，人读到它的时候念当时的数就够了。
 * 这一行只是告知：要不要重新认证由服务端在发布那一刻说了算。
 */
export function AuthCountdown({
  span,
  now,
  onExpire,
  className,
}: {
  /** 读到的那一份近期认证窗口。 */
  span: AuthWindow;
  /** 单调时钟。 */
  now: () => number;
  onExpire: () => void;
  className?: string;
}) {
  const [left, setLeft] = useState(() => remainingMs(span, now()));
  useEffect(() => {
    /** 走一次字；返回到点了没有。 */
    const expired = () => {
      const next = remainingMs(span, now());
      setLeft(next);
      return next <= 0;
    };
    // 到点只说一次，之后不再空转。
    if (expired()) {
      onExpire();
      return;
    }
    const handle = window.setInterval(() => {
      if (!expired()) return;
      window.clearInterval(handle);
      onExpire();
    }, 1000);
    return () => window.clearInterval(handle);
  }, [span, now, onExpire]);
  if (left <= 0) return null;
  return (
    <p className={className}>
      二次认证还有效{' '}
      <span className="policy-countdown__time">{remainingLabel(left)}</span>
      ：这段时间里再批准，一般不用重新输入验证码（以服务端为准）。
    </p>
  );
}
