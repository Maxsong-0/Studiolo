/*
 * 「页面上有没有人在操作」（T-196，live/attention.ts）：一段时间没有操作就算没人在，
 * 自动刷新据此停下，服务端的闲置登出才照常走。
 */
import { describe, expect, it, vi } from 'vitest';
import { ATTENTION_IDLE_MS, createAttention } from '../../live/attention';
import { manualClock } from './manualClock';

describe('有没有人在操作', () => {
  it('满时限没有操作就报没人在；期间有操作则从最后一次操作起重新算，始终只挂一个定时器', async () => {
    const time = manualClock();
    const onChange = vi.fn();
    const attention = createAttention(time.clock, onChange);
    attention.start();
    expect(attention.attended()).toBe(true);

    await time.advance(ATTENTION_IDLE_MS - 1_000);
    attention.touch();
    attention.touch();
    expect(time.pending()).toBe(1);
    await time.advance(ATTENTION_IDLE_MS - 1);
    expect(onChange).not.toHaveBeenCalled();
    expect(time.pending()).toBe(1);

    await time.advance(1);
    expect(onChange).toHaveBeenCalledExactlyOnceWith(false);
    expect(attention.attended()).toBe(false);
    expect(time.pending()).toBe(0);
  });

  it('没人在之后再有操作：立刻报有人在，并重新开始算', async () => {
    const time = manualClock();
    const onChange = vi.fn();
    const attention = createAttention(time.clock, onChange, 60_000);
    attention.start();
    await time.advance(60_000);
    expect(onChange).toHaveBeenLastCalledWith(false);

    attention.touch();
    expect(onChange).toHaveBeenLastCalledWith(true);
    expect(attention.attended()).toBe(true);
    await time.advance(59_999);
    expect(onChange).toHaveBeenCalledTimes(2);
    await time.advance(1);
    expect(onChange).toHaveBeenCalledTimes(3);
    expect(onChange).toHaveBeenLastCalledWith(false);
  });

  it('停下之后不再报；有人在时的操作不重复报', async () => {
    const time = manualClock();
    const onChange = vi.fn();
    const attention = createAttention(time.clock, onChange, 60_000);
    attention.start();
    attention.start();
    expect(time.pending()).toBe(1);
    attention.touch();
    expect(onChange).not.toHaveBeenCalled();

    attention.stop();
    expect(time.pending()).toBe(0);
    await time.advance(10 * 60_000);
    expect(onChange).not.toHaveBeenCalled();
  });
});
