import { useCallback, useLayoutEffect, useRef } from 'react';
import type { RefObject } from 'react';
import { animate, motionValue } from 'motion';

/** 浮层出现与收起的全程时长（规范第 11 节：120ms，与 tokens.css 的 --motion-fast 一致）。 */
const DURATION_S = 0.12;
/** 出现时从下方 4px 升到原位；减少动效时不做位移，只留透明度。 */
const OFFSET_PX = 4;

/**
 * 原生弹窗的呈现、可中断过渡与焦点归还；关闭控件须标记 data-dialog-dismiss。
 * 出现与收起都是 120ms 透明度＋4px 位移（规范第 11 节）；系统要求减少动效时取消位移、只留透明度。
 * 过渡由数值驱动（不依赖 Web Animations），中途反向从当前值接着走，按剩余距离折算时长。
 * 浏览器绕过页面直接关掉对话框（不可取消的关闭请求）时随之收起，onClose 同样只回调一次。
 */
export function useDialogSurface(
  ref: RefObject<HTMLDialogElement | null>,
  onClose: () => void,
): { dismiss: () => void; captureInteraction: () => () => boolean } {
  const interaction = useRef({ revision: 0, active: false });
  const closeCallback = useRef(onClose);
  const controller = useRef<(() => void) | null>(null);
  useLayoutEffect(() => {
    closeCallback.current = onClose;
  }, [onClose]);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const currentInteraction = interaction.current;
    currentInteraction.active = true;
    const previous = document.activeElement;
    const original = {
      opacity: element.style.opacity,
      transform: element.style.transform,
      willChange: element.style.willChange,
    };
    // 无偏好查询能力时保守采用静态呈现（不做任何过渡），不用环境名称区分测试与真实浏览器。
    const preference = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    const animated = Boolean(preference);
    let reduced = preference?.matches ?? true;
    let phase: 'entering' | 'open' | 'exiting' | 'closed' = 'entering';
    let disposed = false;
    let restored = false;
    let revision = 0;
    const progress = motionValue(animated ? 0 : 1);

    // 静止在完全显示时不留变换：恒等的 translateY 也会建层叠上下文与固定定位的包含块。
    // 位移取到 0.01px，免得接近终点时写出 4e-16px 这类科学计数。
    function paint(value: number) {
      const visible = Math.min(1, Math.max(0, value));
      element!.style.opacity = String(visible);
      element!.style.transform =
        reduced || visible >= 1
          ? 'none'
          : `translateY(${Math.round(OFFSET_PX * (1 - visible) * 100) / 100}px)`;
    }

    function restoreFocus() {
      if (restored) return;
      restored = true;
      const active = document.activeElement;
      const otherDialog = [...document.querySelectorAll('dialog[open]')].some(
        (dialog) => dialog !== element,
      );
      if (
        previous instanceof HTMLElement &&
        previous.isConnected &&
        !otherDialog &&
        (active === document.body ||
          active === previous ||
          element!.contains(active))
      ) {
        previous.focus();
      }
    }

    function finish(target: number, currentRevision: number) {
      if (disposed || revision !== currentRevision) return;
      element!.style.willChange = original.willChange;
      if (target === 1) {
        phase = 'open';
        return;
      }
      phase = 'closed';
      if (element!.open) element!.close();
      restoreFocus();
      closeCallback.current();
    }

    function transition(target: number) {
      const currentRevision = ++revision;
      // stop 会补采样当前时刻并保留当前值；反向时从这里接着走，不重设端点。
      progress.stop();
      const distance = Math.abs(target - progress.get());
      if (!animated || distance === 0) {
        progress.jump(target);
        paint(target);
        finish(target, currentRevision);
        return;
      }
      element!.style.willChange = 'transform, opacity';
      void animate(progress, target, {
        type: 'tween',
        ease: 'easeOut',
        duration: DURATION_S * distance,
        onComplete: () => finish(target, currentRevision),
      });
    }

    function dismiss() {
      if (disposed || phase === 'exiting' || phase === 'closed') return;
      phase = 'exiting';
      currentInteraction.revision += 1;
      currentInteraction.active = false;
      transition(0);
    }

    function continueInteraction(event: Event) {
      if (phase !== 'exiting' || !(event.target instanceof Element)) return;
      if (
        event instanceof KeyboardEvent &&
        (['Escape', 'Tab', 'Shift', 'Control', 'Alt', 'Meta'].includes(
          event.key,
        ) ||
          event.metaKey ||
          event.ctrlKey ||
          event.altKey)
      )
        return;
      const control = event.target.closest<HTMLElement>(
        'button, a[href], input, select, textarea, summary, [role="button"], [contenteditable="true"], [tabindex]',
      );
      if (
        !control ||
        control === element ||
        !element!.contains(control) ||
        control.matches(':disabled') ||
        control.closest(
          '[data-dialog-dismiss], [aria-disabled="true"], [inert], [hidden]',
        )
      )
        return;
      phase = 'entering';
      currentInteraction.active = true;
      transition(1);
    }

    // 浏览器绕过页面直接关掉对话框时，close 事件是唯一的信号：关闭请求（Esc、返回手势）拦下一次之后
    // 没有新的用户激活，下一次的 cancel 就不可取消，preventDefault 无效（CloseWatcher 规则）。
    // 不是自己收起引起的就立即跟着收起：作废进行中的过渡、归还焦点、只回调一次 onClose，
    // 不留下看不见却没卸载的对话框。对话框此刻又开着（如严格模式重挂后，前一轮清理排下的 close 才送到）就不是关闭。
    function closedNatively() {
      if (disposed || phase === 'closed' || element!.open) return;
      phase = 'closed';
      revision += 1;
      progress.stop();
      currentInteraction.revision += 1;
      currentInteraction.active = false;
      element!.style.willChange = original.willChange;
      restoreFocus();
      closeCallback.current();
    }

    // 偏好中途改变：立即按新偏好重画当前这一帧（开了减少动效就立刻去掉位移），进行中的过渡照常走完。
    function changePreference(event: MediaQueryListEvent) {
      reduced = event.matches;
      if (disposed || phase === 'closed') return;
      paint(progress.get());
    }

    const unsubscribe = progress.on('change', paint);
    paint(progress.get());
    element.showModal();
    controller.current = dismiss;
    element.addEventListener('pointerdown', continueInteraction);
    element.addEventListener('click', continueInteraction);
    element.addEventListener('keydown', continueInteraction);
    element.addEventListener('close', closedNatively);
    preference?.addEventListener('change', changePreference);
    transition(1);

    return () => {
      disposed = true;
      currentInteraction.revision += 1;
      currentInteraction.active = false;
      revision += 1;
      controller.current = null;
      preference?.removeEventListener('change', changePreference);
      element.removeEventListener('pointerdown', continueInteraction);
      element.removeEventListener('click', continueInteraction);
      element.removeEventListener('keydown', continueInteraction);
      element.removeEventListener('close', closedNatively);
      unsubscribe();
      progress.destroy();
      if (element.open) element.close();
      restoreFocus();
      Object.assign(element.style, original);
    };
  }, [ref]);

  return {
    dismiss: useCallback(() => controller.current?.(), []),
    // 关闭永久使此前异步读取失效；反向继续后，仅新发起的交互可以导航。
    captureInteraction: useCallback(() => {
      const { revision, active } = interaction.current;
      return () =>
        active &&
        interaction.current.active &&
        interaction.current.revision === revision;
    }, []),
  };
}
