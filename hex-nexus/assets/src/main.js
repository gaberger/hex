// Import all CSS
import '../css/chat-layout.css';
import '../css/chat-tokens.css';
import '../css/chat-messages.css';
import '../css/chat-tools.css';
import '../css/chat-sidebar.css';
import '../css/chat-file-panel.css';
import '../css/chat-hljs.css';
import '../css/chat-health.css';
import '../css/chat-dashboard.css';

// Initialize HexChat global
window.HexChat = window.HexChat || {};

// Load JS modules in order
const modules = [
  'chat-utils.js',
  'chat-state.js',
  'chat-ui.js',
  'chat-websocket.js',
  'chat-messages.js',
  'chat-stream.js',
  'chat-sessions.js',
  'chat-sidebar.js',
  'chat-tools.js',
  'chat-file-panel.js',
  'chat-markdown.js',
  'chat-health.js',
  'chat-hexflo.js',
  'chat-dashboard.js',
  'chat-init.js',
];

// Dynamically load scripts
async function loadScripts() {
  for (const mod of modules) {
    try {
      await loadScript(`../js/${mod}`);
    } catch (e) {
      console.error(`Failed to load ${mod}:`, e);
    }
  }
}

function loadScript(src) {
  return new Promise((resolve, reject) => {
    const script = document.createElement('script');
    script.src = src;
    script.onload = resolve;
    script.onerror = reject;
    document.head.appendChild(script);
  });
}

loadScripts();

console.log('[hex-dashboard] Vite dev server ready');
