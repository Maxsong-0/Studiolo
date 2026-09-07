import type { ReactNode } from 'react';
import {
  motion,
  useMotionValue,
  useReducedMotion,
  useSpring,
} from 'motion/react';

// 光线只在指针操作的局部表面移动，正文与整个画布不跟随。
export function PointerSurface({ children }: { children: ReactNode }) {
  const reduced = useReducedMotion();
  const x = useMotionValue(0);
  const y = useMotionValue(0);
  const opacity = useMotionValue(0);
  const lightX = useSpring(x, { stiffness: 400, damping: 40 });
  const lightY = useSpring(y, { stiffness: 400, damping: 40 });
  const lightOpacity = useSpring(opacity, { stiffness: 300, damping: 35 });
  return (
    <div
      className="pointer-surface"
      onPointerMove={(event) => {
        if (reduced || event.pointerType !== 'mouse') return;
        const bounds = event.currentTarget.getBoundingClientRect();
        x.set(event.clientX - bounds.left);
        y.set(event.clientY - bounds.top);
        opacity.set(1);
      }}
      onPointerLeave={() => opacity.set(0)}
    >
      <motion.span
        className="pointer-light"
        aria-hidden="true"
        style={{ x: lightX, y: lightY, opacity: lightOpacity }}
      />
      <div className="pointer-content">{children}</div>
    </div>
  );
}
