import type { ReactNode } from 'react';
import { POLICY_TERMS, type PolicyTermName } from './terms';
import '../styles/policy.css';

/**
 * 人话在前、原术语等宽小字在后：「规则草案 desired」。
 * 给了 children 就用它代替术语表里的人话（如「待处理草案」），原术语照旧跟在后面。
 * 中间的空格是真空格：可访问名读作「规则草案 desired」，不会粘成一个词。
 * 不要放进按钮：按钮的可访问名只用人话。
 */
export function Term({
  name,
  children,
}: {
  name: PolicyTermName;
  children?: ReactNode;
}) {
  const term = POLICY_TERMS[name];
  return (
    <>
      {children ?? term.human}{' '}
      <code className="policy-term" lang="en">
        {term.raw}
      </code>
    </>
  );
}
