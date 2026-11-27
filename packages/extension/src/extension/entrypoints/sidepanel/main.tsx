import '@/lib/zod-locale';
import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { loadCore } from '@/lib/bridge/pure';
import { mountTheme } from '@/lib/bridge/theme';
import '@/extension/assets/globals.css';

mountTheme();

// Schedules and the profile are checked and described by ext-core, synchronously, while rendering.
void loadCore().then(() =>
  ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  ),
);
