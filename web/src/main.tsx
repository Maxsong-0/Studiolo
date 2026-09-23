import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { MotionConfig } from 'motion/react';
import { SessionBoundary } from './SessionBoundary';
// 自托管字体（CSP 只许 font-src 'self'）：只取拉丁子集与规范用到的字重，中文走系统字体。
import '@fontsource/archivo/latin-500.css';
import '@fontsource/archivo/latin-700.css';
import '@fontsource/archivo/latin-800.css';
import '@fontsource/archivo/latin-900.css';
import '@fontsource/ibm-plex-mono/latin-400.css';
import '@fontsource/ibm-plex-mono/latin-500.css';
import '@fontsource/ibm-plex-mono/latin-600.css';
// 只留全局样式：令牌与元素基线、视图共用的全局类、全局窄屏规则。组件私有的样式由组件自己 import；
// session / factor / freshness / health / policy 五份还没有组件自己引入，暂留在这里。
// mobile.css 是视图内部的窄屏与焦点环覆盖，必须最后引入，同特异度时才压得过各视图自己的规则。
import './styles/tokens.css';
import './styles/workspace.css';
import './styles/views.css';
import './styles/responsive.css';
import './styles/session.css';
import './styles/factor.css';
import './styles/freshness.css';
import './styles/health.css';
import './styles/policy.css';
import './styles/mobile.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <MotionConfig reducedMotion="user">
      <SessionBoundary>
        {(account) => <App account={account} />}
      </SessionBoundary>
    </MotionConfig>
  </StrictMode>,
);
