const NEXUS = '';
const WS_BASE = location.protocol === 'https:' ? 'wss:' : 'ws:';
let ws: WebSocket | null = null;
let sessionId: string | null = null;
let totalIn = 0;
let totalOut = 0;

function $(id: string): HTMLElement | null {
  return document.getElementById(id);
}

function clearChildren(el: HTMLElement): void {
  while (el.firstChild) el.removeChild(el.firstChild);
}

function mkEmpty(text: string): HTMLDivElement {
  const d = document.createElement('div');
  d.className = 'empty';
  d.textContent = text;
  return d;
}

function fmtTokens(n: number): string {
  return n >= 1e6 ? (n / 1e6).toFixed(1) + 'M' : n >= 1e3 ? (n / 1e3).toFixed(1) + 'K' : '' + n;
}

function connect(sid?: string): void {
  if (ws) ws.close();
  let url = WS_BASE + '//' + location.host + '/ws/chat';
  if (sid) url += '?session_id=' + sid;
  ws = new WebSocket(url);
  ws.onopen = () => {
    $('connDot')?.classList.add('ok');
    const connLabel = $('connLabel');
    if (connLabel) connLabel.textContent = 'connected';
    const nexusStatus = $('nexusStatus');
    if (nexusStatus) nexusStatus.textContent = 'connected';
  };
  ws.onclose = () => {
    $('connDot')?.classList.remove('ok');
    const connLabel = $('connLabel');
    if (connLabel) connLabel.textContent = 'disconnected';
    const nexusStatus = $('nexusStatus');
    if (nexusStatus) nexusStatus.textContent = 'disconnected';
  };
  ws.onmessage = (e) => {
    try {
      const env = JSON.parse(e.data);
      if (env.event === 'connected') {
        sessionId = env.data.persistentSessionId || env.data.sessionId;
        const sessionLabel = $('sessionLabel');
        if (sessionLabel) sessionLabel.textContent = sessionId ? sessionId.slice(0, 8) + '...' : '--';
        loadSessions();
      } else if (env.event === 'agent_status') {
        const status = env.data.status || 'idle';
        if (status === 'thinking') {
          showThinking(true);
        } else {
          showThinking(false);
        }
        loadAgents();
      } else if (env.event === 'chat_message') {
        showThinking(false);
        addMsg('assistant', env.data.content);
        loadSessions();
      } else if (env.event === 'token_update') {
        totalIn += env.data.input_tokens || 0;
        totalOut += env.data.output_tokens || 0;
        const tokenCount = $('tokenCount');
        if (tokenCount) tokenCount.textContent = fmtTokens(totalIn + totalOut);
        const tokenInfo = $('tokenInfo');
        if (tokenInfo) tokenInfo.textContent = 'In: ' + fmtTokens(totalIn) + '  Out: ' + fmtTokens(totalOut);
      } else if (env.event === 'agent_connected') {
        loadAgents();
      }
    } catch (err) { /* ignore */ }
  };
}

function addMsg(role: string, content: string): void {
  const messages = $('messages');
  if (!messages) return;
  const d = document.createElement('div');
  d.className = 'msg';
  const s = document.createElement('span');
  s.className = 'sender ' + role;
  s.textContent = role === 'user' ? 'You: ' : role === 'assistant' ? 'Assistant: ' : role + ': ';
  d.appendChild(s);
  d.appendChild(document.createTextNode(content));
  messages.appendChild(d);
  messages.scrollTop = messages.scrollHeight;
}

function send(): void {
  const input = $('chatInput') as HTMLInputElement | null;
  const text = input?.value.trim();
  if (!text || !ws) return;
  ws.send(JSON.stringify({ type: 'chat_message', content: text }));
  addMsg('user', text);
  if (input) input.value = '';
  showThinking(true);
}

let thinkingEl: HTMLDivElement | null = null;
function showThinking(on: boolean): void {
  const sendBtn = $('sendBtn') as HTMLButtonElement | null;
  const chatInput = $('chatInput') as HTMLInputElement | null;
  if (sendBtn) sendBtn.disabled = on;
  if (chatInput) chatInput.disabled = on;
  if (on) {
    if (!thinkingEl) {
      thinkingEl = document.createElement('div');
      thinkingEl.className = 'msg';
      thinkingEl.style.color = '#8b949e';
      thinkingEl.style.fontStyle = 'italic';
      thinkingEl.textContent = 'Thinking...';
      const messages = $('messages');
      if (messages) {
        messages.appendChild(thinkingEl);
        messages.scrollTop = messages.scrollHeight;
      }
    }
  } else {
    if (thinkingEl) {
      thinkingEl.remove();
      thinkingEl = null;
    }
    if (sendBtn) sendBtn.disabled = false;
    if (chatInput) {
      chatInput.disabled = false;
      chatInput.focus();
    }
  }
}

$('sendBtn')?.addEventListener('click', send);
$('chatInput')?.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !(e as KeyboardEvent).shiftKey) {
    e.preventDefault();
    send();
  }
});

function forkSession(sid: string): void {
  fetch(NEXUS + '/api/sessions/' + sid + '/fork', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' })
    .then(() => loadSessions())
    .catch(() => {});
}

function compactSession(sid: string): void {
  fetch(NEXUS + '/api/sessions/' + sid + '/compact', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ summary: 'Compacted session' }) })
    .then(() => loadSessions())
    .catch(() => {});
}

function loadSessions(): void {
  const el = $('sessionList');
  if (!el) return;
  fetch(NEXUS + '/api/sessions?project_id=&limit=20')
    .then(r => r.json())
    .then((sessions: any[]) => {
      clearChildren(el);
      if (!sessions.length) { el.appendChild(mkEmpty('No sessions')); return; }
      sessions.forEach((s) => {
        const d = document.createElement('div');
        d.className = 'session' + (s.id === sessionId ? ' active' : '');

        const label = document.createElement('span');
        label.textContent = (s.title || 'Untitled').slice(0, 30) + ' (' + s.messageCount + ' msgs)';
        d.appendChild(label);

        const actions = document.createElement('span');
        actions.style.cssText = 'float:right;opacity:0;transition:opacity 0.15s;';

        const forkBtn = document.createElement('button');
        forkBtn.textContent = 'fork';
        forkBtn.style.cssText = 'background:none;border:none;color:#58a6ff;cursor:pointer;font-size:10px;margin-left:4px;';
        forkBtn.addEventListener('click', (e) => { e.stopPropagation(); forkSession(s.id); });

        const compactBtn = document.createElement('button');
        compactBtn.textContent = 'compact';
        compactBtn.style.cssText = 'background:none;border:none;color:#f0883e;cursor:pointer;font-size:10px;margin-left:4px;';
        compactBtn.addEventListener('click', (e) => { e.stopPropagation(); compactSession(s.id); });

        actions.appendChild(forkBtn);
        actions.appendChild(compactBtn);
        d.appendChild(actions);

        d.addEventListener('mouseenter', () => { actions.style.opacity = '1'; });
        d.addEventListener('mouseleave', () => { actions.style.opacity = '0'; });

        d.addEventListener('click', () => {
          connect(s.id);
          const msgs = $('messages');
          if (msgs) clearChildren(msgs);
          loadMessages(s.id);
        });
        el.appendChild(d);
      });
    })
    .catch(() => {});
}

function loadMessages(sid: string): void {
  fetch(NEXUS + '/api/sessions/' + sid + '/messages?limit=100')
    .then(r => r.json())
    .then((msgs: any[]) => {
      msgs.forEach((m) => {
        const text = (m.parts || []).map((p: any) => p.content || p.tool_name || '').join('');
        if (text) addMsg(m.role, text);
      });
    })
    .catch(() => {});
}

function loadAgents(): void {
  const el = $('agentList');
  const agentCount = $('agentCount');
  if (!el) return;
  fetch(NEXUS + '/api/agents')
    .then(r => r.json())
    .then((agents: any[]) => {
      if (agentCount) agentCount.textContent = '' + agents.length;
      clearChildren(el);
      if (!agents.length) { el.appendChild(mkEmpty('No agents')); return; }
      agents.forEach((a) => {
        const d = document.createElement('div');
        d.className = 'agent';
        const dot = document.createElement('span');
        dot.className = 'status-dot ' + (a.status === 'running' ? 'running' : 'idle');
        d.appendChild(dot);
        const name = document.createElement('span');
        name.className = 'name';
        name.textContent = a.name;
        d.appendChild(name);
        const meta = document.createElement('div');
        meta.className = 'meta';
        meta.textContent = a.model || '';
        d.appendChild(meta);
        el.appendChild(d);
      });
    })
    .catch(() => {});
}

function loadSwarms(): void {
  const el = $('swarmList');
  if (!el) return;
  fetch(NEXUS + '/api/swarms/active')
    .then(r => r.json())
    .then((swarms: any[]) => {
      clearChildren(el);
      if (!swarms.length) { el.appendChild(mkEmpty('No active swarms')); return; }
      swarms.forEach((s) => {
        const d = document.createElement('div');
        d.className = 'agent';
        const name = document.createElement('span');
        name.className = 'name';
        name.textContent = s.name;
        d.appendChild(name);
        const meta = document.createElement('div');
        meta.className = 'meta';
        meta.textContent = s.status;
        d.appendChild(meta);
        el.appendChild(d);
      });
    })
    .catch(() => {});
}

function loadInference(): void {
  const el = $('inferenceList');
  const inferenceCount = $('inferenceCount');
  if (!el) return;
  fetch(NEXUS + '/api/inference/endpoints')
    .then(r => r.json())
    .then((endpoints: any) => {
      clearChildren(el);
      const list = Array.isArray(endpoints) ? endpoints : Object.values(endpoints);
      if (inferenceCount) inferenceCount.textContent = '' + list.length;
      if (!list.length) { el.appendChild(mkEmpty('No providers registered. Use: hex inference add <provider> <url>')); return; }

      const byProvider: Record<string, any[]> = {};
      list.forEach((ep: any) => {
        const key = ep.provider || 'unknown';
        if (!byProvider[key]) byProvider[key] = [];
        byProvider[key].push(ep);
      });

      Object.keys(byProvider).forEach((provider) => {
        const header = document.createElement('div');
        header.style.cssText = 'font-size:11px;color:#58a6ff;font-weight:600;margin:8px 0 4px;text-transform:uppercase;';
        header.textContent = provider;
        el.appendChild(header);

        byProvider[provider].forEach((ep) => {
          const d = document.createElement('div');
          d.className = 'agent';

          const dot = document.createElement('span');
          const isHealthy = ep.status === 'healthy' || ep.status === 'ok';
          dot.className = 'status-dot ' + (isHealthy ? 'running' : 'idle');
          d.appendChild(dot);

          const name = document.createElement('span');
          name.className = 'name';
          name.textContent = ep.model || ep.id || 'default';
          d.appendChild(name);

          const urlMeta = document.createElement('div');
          urlMeta.className = 'meta';
          urlMeta.textContent = ep.url || '';
          d.appendChild(urlMeta);

          const statusMeta = document.createElement('div');
          statusMeta.className = 'meta';
          statusMeta.style.color = isHealthy ? '#3fb950' : '#f85149';
          statusMeta.textContent = ep.status || 'unknown';
          if (ep.health_checked_at) {
            statusMeta.textContent += ' (' + timeAgo(ep.health_checked_at) + ')';
          }
          d.appendChild(statusMeta);

          if (provider === 'ollama' && ep.url) {
            fetchOllamaModels(ep.url, d);
          }

          el.appendChild(d);
        });
      });
    })
    .catch(() => {});
}

function fetchOllamaModels(baseUrl: string, parentEl: HTMLElement): void {
  fetch(baseUrl + '/api/tags')
    .then(r => r.json())
    .then((data) => {
      const models = data.models || [];
      if (!models.length) return;
      const modelsDiv = document.createElement('div');
      modelsDiv.className = 'meta';
      modelsDiv.style.cssText = 'margin-left:12px;color:#8b949e;';
      const names = models.map((m: any) => m.name).slice(0, 8);
      modelsDiv.textContent = 'Models: ' + names.join(', ');
      if (models.length > 8) modelsDiv.textContent += ' (+' + (models.length - 8) + ' more)';
      parentEl.appendChild(modelsDiv);
    })
    .catch(() => {});
}

function timeAgo(isoStr: string): string {
  if (!isoStr) return '';
  const secs = Math.floor((Date.now() - new Date(isoStr).getTime()) / 1000);
  if (secs < 60) return secs + 's ago';
  if (secs < 3600) return Math.floor(secs / 60) + 'm ago';
  if (secs < 86400) return Math.floor(secs / 3600) + 'h ago';
  return Math.floor(secs / 86400) + 'd ago';
}

function loadRlStats(): void {
  fetch(NEXUS + '/api/rl/stats')
    .then(r => r.json())
    .then((stats) => {
      if (stats && stats.total_experiences !== undefined) {
        const info = $('tokenInfo');
        if (info) {
          const existing = info.textContent;
          const rl = 'RL: ' + stats.total_experiences + ' exp, e=' + (stats.epsilon || 0).toFixed(2);
          info.textContent = existing === '--' ? rl : existing + ' | ' + rl;
        }
      }
    })
    .catch(() => {});
}

// Boot
connect();
loadAgents();
loadSwarms();
loadInference();
loadRlStats();
setInterval(loadAgents, 5000);
setInterval(loadSwarms, 10000);
setInterval(loadInference, 10000);
