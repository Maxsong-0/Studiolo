import { useCallback, useLayoutEffect, useRef } from 'react';
import type { RefObject } from 'react';
import { animate, motionValue } from 'motion';

/** 原生弹窗的即时呈现、可中断过渡与焦点归还；关闭控件须标记 data-dialog-dismiss。 */
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
    // 无偏好查询能力时保守采用静态呈现，不用环境名称区分测试与真实浏览器。
    const preference = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    let reduced = preference?.matches ?? true;
    let phase: 'entering' | 'open' | 'exiting' | 'closed' = 'entering';
    let disposed = false;
    let restored = false;
    let revision = 0;
    const progress = motionValue(reduced ? 1 : 0);

    function paint(value: number) {
      element!.style.opacity = String(Math.min(1, Math.max(0, value)));
      element!.style.transform = reduced
        ? 'none'
        : `translateY(${12 * (1 - value)}px) scale(${0.97 + 0.03 * value})`;
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
      // stop 会补采样当前时刻并保留值与速度；反向不重设端点。
      progress.stop();
      if (reduced) {
        progress.jump(target);
        paint(target);
        finish(target, currentRevision);
        return;
      }
      element!.style.willChange = 'transform, opacity';
      void animate(progress, target, {
        type: 'spring',
        stiffness: 500,
        damping: 2 * Math.sqrt(500),
        mass: 1,
        restDelta: 0.001,
        restSpeed: 0.01,
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

    function changePreference(event: MediaQueryListEvent) {
      reduced = event.matches;
      if (disposed || phase === 'closed') return;
      paint(progress.get());
      transition(phase === 'exiting' ? 0 : 1);
    }

    const unsubscribe = progress.on('change', paint);
    paint(progress.get());
    element.showModal();
    controller.current = dismiss;
    element.addEventListener('pointerdown', continueInteraction);
    element.addEventListener('click', continueInteraction);
    element.addEventListener('keydown', continueInteraction);
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
