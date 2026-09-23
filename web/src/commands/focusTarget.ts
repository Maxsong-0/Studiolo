/**
 * ⌘K 动作面板的「带路」：不可逆动作不在面板里执行，只把人带到拥有该动作的界面，
 * 再把焦点放到那个控件上（设计规范第 8 节）。确认仍在原界面完成。
 *
 * 约定：拥有动作的界面在对应控件（或 tabIndex={-1} 的容器）上写
 * `data-command-target="<下表的值>"`。没写或还没渲染出来，就不动焦点——
 * 面板关闭时焦点已还给 ⌘K 入口，人照样到了那个界面。
 */
export const COMMAND_TARGET_ATTRIBUTE = 'data-command-target';

export const COMMAND_TARGETS = {
  /** 策略页「待处理 desired」队列：发布前要在这里选定草案。 */
  policyQueue: 'policy-desired-queue',
  /** 策略页「版本与回滚」列表：回滚在这里逐版确认。 */
  policyVersions: 'policy-versions',
  /** 文档页的搜索框：删文件要先找到并打开那个文件。 */
  documentsSearch: 'documents-search',
  /** 协作 › 执行里的「代码候选落地」：核对 HEAD 与补丁摘要后二次认证落地。 */
  executionLanding: 'execution-landing',
  /** 协作 › 执行里的「逐项审阅候选」：候选产物在这里逐项审阅。 */
  executionReview: 'execution-review',
} as const;

export type CommandTarget =
  (typeof COMMAND_TARGETS)[keyof typeof COMMAND_TARGETS];

/** 目标视图是异步渲染的：按帧等它出现，最多等这么久。 */
const FOCUS_DEADLINE_MS = 2000;

function nextFrame(callback: () => void): () => void {
  if (typeof window.requestAnimationFrame === 'function') {
    const frame = window.requestAnimationFrame(callback);
    return () => window.cancelAnimationFrame(frame);
  }
  const timer = window.setTimeout(callback, 16);
  return () => window.clearTimeout(timer);
}

/**
 * 跳转之后把焦点交给目标控件。第一帧时面板已经关掉、焦点已还给入口按钮，
 * 以那一刻的焦点为基准：之后焦点只要变了（人自己动了）、或有别的对话框开着，就放弃，绝不抢焦点。
 * 返回取消函数。
 */
export function focusCommandTarget(target: CommandTarget): () => void {
  const selector = `[${COMMAND_TARGET_ATTRIBUTE}="${target}"]`;
  const deadline = Date.now() + FOCUS_DEADLINE_MS;
  let baseline: Element | null | undefined;
  let cancel = nextFrame(step);

  function step() {
    if (baseline === undefined) baseline = document.activeElement;
    else if (document.activeElement !== baseline) return;
    if (document.querySelector('dialog[open]')) return;
    const element = document.querySelector<HTMLElement>(selector);
    if (element) {
      element.focus();
      return;
    }
    if (Date.now() < deadline) cancel = nextFrame(step);
  }

  return () => cancel();
}
