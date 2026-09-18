import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { MotionConfig } from 'motion/react';
import { SessionBoundary } from './SessionBoundary';
import './styles/tokens.css';
import './styles/workbench.css';
import './styles/workspace.css';
import './styles/projects.css';
import './styles/inbox.css';
import './styles/views.css';
import './styles/document.css';
import './styles/search.css';
import './styles/navigation.css';
import './styles/tree.css';
import './styles/responsive.css';
import './styles/projects-mobile.css';
import './styles/symbols.css';
import './styles/file-icons.css';
import './styles/session.css';
import './styles/factor.css';
import './styles/operations.css';
import './styles/architecture.css';
import './styles/cargo-dependencies.css';
import './styles/rust-call-sites.css';
import './styles/rust-call-links.css';
import './styles/freshness.css';
import './styles/editor.css';
import './styles/history.css';
import './styles/file-operations.css';
import './styles/health.css';
import './styles/policy.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <MotionConfig reducedMotion="user">
      <SessionBoundary>
        {(account) => <App account={account} />}
      </SessionBoundary>
    </MotionConfig>
  </StrictMode>,
);
