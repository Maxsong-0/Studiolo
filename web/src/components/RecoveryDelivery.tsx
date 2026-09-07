import { useEffect, useRef, useState } from 'react';
import { Icon } from './Icon';

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
      <p>
        关闭账户、切换页面或离开当前标签页后，这里会清除恢复码，无法重新读取。
      </p>
      <ol className="recovery-codes" aria-label="新恢复码">
        {codes.map((code, index) => (
          <li key={index}>
            <code>{code}</code>
          </li>
        ))}
      </ol>
      {message ? <p role="status">{message}</p> : null}
      <div className="factor-actions">
        <button className="outline-button" onClick={() => void copy()}>
          <Icon name="copy" />
          复制恢复码
        </button>
        <button className="outline-button" onClick={onClose}>
          我已保存，收起恢复码
        </button>
      </div>
    </section>
  );
}
