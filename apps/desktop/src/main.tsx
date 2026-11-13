import '@fontsource/inter/400.css';
import '@fontsource/inter/500.css';
import '@fontsource/inter/600.css';
import '@fontsource/inter/700.css';
import '@fontsource/space-grotesk/500.css';
import '@fontsource/space-grotesk/600.css';
import '@fontsource/space-grotesk/700.css';
import './theme/tokens.css';
import './app/app.css';

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { createMockMailApi } from './api/mockMailApi';
import { App } from './app/App';

const root = document.getElementById('root');
if (!root) throw new Error('Missing #root element');

// The Tauri-backed MailApi replaces the mock once the Rust commands land.
createRoot(root).render(
  <StrictMode>
    <App api={createMockMailApi()} />
  </StrictMode>,
);
