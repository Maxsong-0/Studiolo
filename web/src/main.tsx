import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { MotionConfig } from 'motion/react';
import { SessionBoundary } from './SessionBoundary';
import { DARK_SCHEME_QUERY, startTabIcon } from './brand/tabIcon';
import { TabIconContext } from './brand/tabIconContext';
import { startTheme } from './theme/preference';
// 自托管字体（CSP 只许 font-src 'self'）：只取拉丁子集与规范用到的字重，中文走系统字体。
import './styles/fonts.css';
// 只留全局样式：令牌与元素基线、视图共用的全局类、全局窄屏规则。组件私有的样式由组件自己 import；
// mobile.css 是视图内部的窄屏与焦点环覆盖，必须最后引入，同特异度时才压得过各视图自己的规则。
import './styles/tokens.css';
import './styles/workspace.css';
import './styles/views.css';
import './styles/responsive.css';
import './styles/mobile.css';

// 手动选的深浅色（T-199）：首屏之前已由 /assets/theme-boot-*.js 套过一次；这里再套一次并跟着之后的改动走，
// 启动脚本没跑成（本机存储读不了、文件被拦）时由这里补上。登录页也算。
startTheme(document);

// 标签页图标从这里起就跟着系统深浅色走，登录页也算；有没有事等你由外壳报上来（设计规范第 14 节）。
// 它不跟手动选的深浅色：标签栏的颜色是浏览器的，不是页面的。
const tabIcon = startTabIcon(document, window.matchMedia?.(DARK_SCHEME_QUERY));

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <TabIconContext.Provider value={tabIcon}>
      <MotionConfig reducedMotion="user">
        <SessionBoundary>
          {(account) => <App account={account} />}
        </SessionBoundary>
      </MotionConfig>
    </TabIconContext.Provider>
  </StrictMode>,
);
