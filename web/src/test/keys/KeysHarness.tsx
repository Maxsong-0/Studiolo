import { useState, type ReactNode } from 'react';
import { KeyboardSheet } from '../../keys/KeyboardSheet';
import type { SingleKeysPreference } from '../../keys/preference';
import { KeyScopeContext, useGlobalKeys } from '../../keys/useGlobalKeys';
import type { View } from '../../types';

/**
 * 不渲染整个 App、单独渲染某个视图时，由这里代行 App 的那一份全站按键监听和快捷键一览（T-197）：
 * 同一个 useGlobalKeys、同一个 KeyboardSheet，只是跳转交给测试自己接。
 */
export function KeysHarness({
  children,
  preference,
  onGo = () => {},
}: {
  children: ReactNode;
  /** 单键快捷键的开关：每例各用各的，不碰全站那一个。 */
  preference: SingleKeysPreference;
  onGo?: (view: View) => void;
}) {
  const [sheet, setSheet] = useState(false);
  const keys = useGlobalKeys({
    onGo,
    onShowSheet: () => setSheet(true),
    preference,
  });
  return (
    <KeyScopeContext.Provider value={keys}>
      {children}
      {sheet ? (
        <KeyboardSheet
          preference={keys.preference}
          onClose={() => setSheet(false)}
        />
      ) : null}
    </KeyScopeContext.Provider>
  );
}
