import '@fontsource/inter/400.css';
import '@fontsource/inter/500.css';
import '@fontsource/inter/600.css';
import '@fontsource/inter/700.css';
import '@fontsource/fraunces/600.css';
import './theme/tokens.css';
import './app/app.css';

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { createMockMailApi } from './api/mock/mockMailApi';
import { createTauriMailApi } from './api/tauriMailApi';
import { App } from './app/App';
import { composeWindowKey } from './features/compose/ComposeWindow';
import { messageWindowId } from './features/reader/MessageWindow';

const root = document.getElementById('root');
if (!root) throw new Error('Missing #root element');

// Inside the desktop app talk to the Rust backend; in a plain browser
// (vite dev) use the in-memory mock so the interface can be worked on alone.
const api = '__TAURI_INTERNALS__' in window ? createTauriMailApi() : createMockMailApi({ now: () => Math.floor(Date.now() / 1000), simulateDelivery: true });

createRoot(root).render(
  <StrictMode>
    <App api={api} messageId={messageWindowId(window.location.search)} composeKey={composeWindowKey(window.location.search)} />
  </StrictMode>,
);
