import type { Feedback } from './usePolicyActions';
import '../styles/policy.css';

/** 动作结果落在发起它的那一块：人在哪里点的，就在哪里看结果（与策略页同一写法）。 */
export function RulesFeedback({
  origin,
  notice,
  error,
}: {
  origin: string;
  notice: Feedback | null;
  error: Feedback | null;
}) {
  return (
    <>
      {notice?.origin === origin ? (
        <p className="policy-feedback" role="status">
          {notice.text}
        </p>
      ) : null}
      {error?.origin === origin ? (
        <p className="policy-feedback policy-feedback--error" role="alert">
          {error.text}
        </p>
      ) : null}
    </>
  );
}
