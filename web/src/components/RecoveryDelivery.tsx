import { useEffect, useRef, useState } from 'react';
import { Button } from '../ui';
import { Icon } from './Icon';
import '../styles/factor.css';

/**
 * 新恢复码只交付这一次：等宽网格，每码一格、1px 墨线分隔，可以直接选中；
 * 只有点「复制恢复码」才写剪贴板。收起即清除，不能重读。
 */
export function RecoveryDelivery({
  codes,
  onClose,
}: {
  codes: string[];
  onClose: () => void;
}) {
  const [message, setMessage] = useState('');
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  async function copy() {
    try {
      if (!navigator.clipboard) throw new Error();
      await navigator.clipboard.writeText(codes.join('\n'));
      if (mounted.current) setMessage('已复制，请保存到安全的位置。');
    } catch {
      if (mounted.current) setMessage('未能复制。可以选中下方恢复码自行保存。');
    }
  }
  return (
    <section className="recovery-delivery" aria-labelledby="delivery-title">
      <h4 id="delivery-title">请保存新的恢复码</h4>
      <p>之前的恢复码已失效。这十枚新码仅本次显示；每枚只能使用一次。</p>
      <p className="factor-warning">
        关闭账户、切换页面或离开当前标签页后，这里会清除恢复码，无法重新读取。
      </p>
      {/* role=list：去掉列表符号后 Safari 会丢掉列表语义，显式补回。 */}
      <ol className="recovery-codes" role="list" aria-label="新恢复码">
        {codes.map((code, index) => (
          <li key={index}>
            <code>{code}</code>
          </li>
        ))}
      </ol>
      {message ? (
        <p className="factor-note" role="status">
          {message}
        </p>
      ) : null}
      <div className="factor-actions">
        <Button size="sm" onClick={() => void copy()}>
          <Icon name="copy" />
          复制恢复码
        </Button>
        <Button variant="primary" size="sm" onClick={onClose}>
          我已保存，收起恢复码
        </Button>
      </div>
    </section>
  );
}
