/* Session management — persistence, list, switching */
(function(H) {
"use strict";
var state = H.state;

/* -- Session state -- */
state.persistentSessionId = null;
state.sessions = [];
state.currentProjectId = null;
state.sessionListContainer = null;

/* -- API helpers -- */

function apiCall(method, path, body) {
  var opts = { method: method, headers: { "Content-Type": "application/json" } };
  if (state.authToken) opts.headers["Authorization"] = "Bearer " + state.authToken;
  if (body) opts.body = JSON.stringify(body);
  return fetch(path, opts).then(function(r) {
    if (!r.ok) throw new Error("Session API " + r.status);
    if (r.status === 204) return null;
    return r.json();
  });
}

/* -- Session API client -- */

function fetchSessions(projectId) {
  var q = projectId ? "?project_id=" + encodeURIComponent(projectId) : "";
  return apiCall("GET", "/api/sessions" + q);
}

function createSession(projectId, model) {
  var body = {};
  if (projectId) body.project_id = projectId;
  if (model) body.model = model;
  return apiCall("POST", "/api/sessions", body);
}

function fetchMessages(sessionId, limit) {
  var q = limit ? "?limit=" + limit : "";
  return apiCall("GET", "/api/sessions/" + encodeURIComponent(sessionId) + "/messages" + q);
}

function updateTitle(sessionId, title) {
  return apiCall("PATCH", "/api/sessions/" + encodeURIComponent(sessionId), { title: title });
}

function deleteSession(sessionId) {
  return apiCall("DELETE", "/api/sessions/" + encodeURIComponent(sessionId));
}

function forkSession(sessionId) {
  return apiCall("POST", "/api/sessions/" + encodeURIComponent(sessionId) + "/fork");
}

/* -- Time formatting -- */

function timeAgo(isoStr) {
  if (!isoStr) return "";
  var diff = Math.floor((Date.now() - new Date(isoStr).getTime()) / 1000);
  if (diff < 60) return "just now";
  if (diff < 3600) return Math.floor(diff / 60) + "m ago";
  if (diff < 86400) return Math.floor(diff / 3600) + "h ago";
  if (diff < 604800) return Math.floor(diff / 86400) + "d ago";
  return new Date(isoStr).toLocaleDateString();
}

/* -- Session list renderer -- */

function renderSessionList(sessions, currentSessionId, container) {
  if (!container) return;

  /* Clear container using DOM API */
  while (container.firstChild) {
    container.removeChild(container.firstChild);
  }

  if (!sessions || sessions.length === 0) {
    var empty = document.createElement("div");
    empty.className = "session-empty";
    empty.textContent = "No sessions yet";
    container.appendChild(empty);
    return;
  }

  var list = document.createElement("div");
  list.className = "session-list";

  for (var i = 0; i < sessions.length; i++) {
    var s = sessions[i];
    var item = document.createElement("div");
    item.className = "session-item" + (s.id === currentSessionId ? " session-active" : "");
    item.dataset.sessionId = s.id;

    var info = document.createElement("div");
    info.className = "session-info";
    info.addEventListener("click", (function(sid) {
      return function() { H.switchSession(sid); };
    })(s.id));

    var title = document.createElement("div");
    title.className = "session-title";
    title.textContent = H.truncate(s.title || "New conversation", 32);
    info.appendChild(title);

    var meta = document.createElement("div");
    meta.className = "session-meta";
    var count = s.message_count != null ? s.message_count : 0;
    meta.textContent = count + " msg" + (count !== 1 ? "s" : "") + " \u00B7 " + timeAgo(s.updated_at || s.created_at);
    info.appendChild(meta);

    item.appendChild(info);

    var actions = document.createElement("div");
    actions.className = "session-actions";

    var forkBtn = document.createElement("button");
    forkBtn.className = "session-btn session-fork-btn";
    forkBtn.title = "Fork session";
    forkBtn.textContent = "\u2442";
    forkBtn.addEventListener("click", (function(sid) {
      return function(e) { e.stopPropagation(); H.forkAndSwitch(sid); };
    })(s.id));
    actions.appendChild(forkBtn);

    var delBtn = document.createElement("button");
    delBtn.className = "session-btn session-del-btn";
    delBtn.title = "Delete session";
    delBtn.textContent = "\u2715";
    delBtn.addEventListener("click", (function(sid) {
      return function(e) { e.stopPropagation(); H.confirmDeleteSession(sid); };
    })(s.id));
    actions.appendChild(delBtn);

    item.appendChild(actions);
    list.appendChild(item);
  }

  container.appendChild(list);
}

/* -- Session switching -- */

function switchSession(sessionId) {
  if (sessionId === state.persistentSessionId) return;

  /* Close current WebSocket and reconnect with session param */
  if (state.ws) {
    state.ws.onclose = null; /* prevent auto-reconnect during intentional close */
    state.ws.close();
  }

  state.persistentSessionId = sessionId;

  /* Reconnect with session_id query param */
  var proto = location.protocol === "https:" ? "wss:" : "ws:";
  var host = location.host || "localhost:5555";
  var url = proto + "//" + host + "/ws/chat?token=" + encodeURIComponent(state.authToken);
  url += "&session_id=" + encodeURIComponent(sessionId);

  var ws = new WebSocket(url);
  state.ws = ws;

  ws.onopen = function() {
    state.connected = true;
    state.reconnectDelay = 1000;
    var dot = document.getElementById("connDot");
    var label = document.getElementById("connLabel");
    if (dot) dot.classList.remove("disconnected");
    if (label) label.textContent = "connected";
  };

  ws.onclose = function() {
    state.connected = false;
    var dot = document.getElementById("connDot");
    var label = document.getElementById("connLabel");
    if (dot) dot.classList.add("disconnected");
    if (label) label.textContent = "disconnected";
  };

  ws.onerror = function() { ws.close(); };

  ws.onmessage = function(e) {
    try {
      var raw = JSON.parse(e.data);
      var msg;
      if (raw.event && raw.data) {
        msg = Object.assign({}, raw.data, { type: raw.event });
      } else {
        msg = raw;
      }
      H.handleMessage(msg);
    } catch(err) { console.error("ws parse error", err); }
  };

  /* Clear current messages and load history */
  if (H.clearMessages) H.clearMessages();
  loadSessionMessages(sessionId);
  refreshSessionList();
}

function loadSessionMessages(sessionId) {
  fetchMessages(sessionId, 100).then(function(data) {
    var msgs = data && data.messages ? data.messages : (Array.isArray(data) ? data : []);
    for (var i = 0; i < msgs.length; i++) {
      var m = msgs[i];
      if (m.role === "user" && H.addUserMessage) {
        H.addUserMessage(m.content, true);
      } else if (m.role === "assistant" && H.addAssistantMessage) {
        H.addAssistantMessage(m.content, true);
      } else if (m.role === "system" && H.addSystemMessage) {
        H.addSystemMessage(m.content);
      }
    }
  }).catch(function(err) {
    console.error("Failed to load session messages", err);
  });
}

/* -- Fork and switch -- */

function forkAndSwitch(sessionId) {
  forkSession(sessionId).then(function(data) {
    if (data && data.id) {
      H.switchSession(data.id);
    }
    refreshSessionList();
  }).catch(function(err) {
    console.error("Failed to fork session", err);
  });
}

/* -- Delete with confirmation -- */

function confirmDeleteSession(sessionId) {
  if (!confirm("Delete this session? This cannot be undone.")) return;
  deleteSession(sessionId).then(function() {
    /* If we deleted the active session, clear state */
    if (sessionId === state.persistentSessionId) {
      state.persistentSessionId = null;
      if (H.clearMessages) H.clearMessages();
    }
    refreshSessionList();
  }).catch(function(err) {
    console.error("Failed to delete session", err);
  });
}

/* -- Auto-title from first message -- */

function maybeAutoTitle(content) {
  if (!state.persistentSessionId) return;
  /* Only auto-title if this is the first message (turn 1) */
  if (state.turnCount !== 1) return;

  var title = content.slice(0, 60).replace(/\n/g, " ").trim();
  if (!title) return;

  updateTitle(state.persistentSessionId, title).then(function() {
    refreshSessionList();
  }).catch(function() {});
}

/* -- Refresh session list -- */

function refreshSessionList() {
  fetchSessions(state.currentProjectId).then(function(data) {
    var sessions = data && data.sessions ? data.sessions : (Array.isArray(data) ? data : []);
    state.sessions = sessions;
    renderSessionList(sessions, state.persistentSessionId, state.sessionListContainer);
  }).catch(function(err) {
    console.error("Failed to fetch sessions", err);
  });
}

/* -- Handle welcome message with persistentSessionId -- */

function handleSessionWelcome(msg) {
  if (msg.persistentSessionId) {
    state.persistentSessionId = msg.persistentSessionId;
    refreshSessionList();
  }
}

/* -- New session button -- */

function createNewSession() {
  var model = null;
  var badge = document.getElementById("modelBadge");
  if (badge) model = badge.textContent;
  createSession(state.currentProjectId, model).then(function(data) {
    if (data && data.id) {
      H.switchSession(data.id);
    }
  }).catch(function(err) {
    console.error("Failed to create session", err);
  });
}

/* -- Initialize session manager -- */

function initSessionManager() {
  /* Find or create the session list container in the sidebar */
  var sidebar = document.getElementById("sidebar");
  if (!sidebar) return;

  /* Insert session section before the first existing section */
  var section = document.createElement("div");
  section.className = "sb-section session-section";

  var titleRow = document.createElement("div");
  titleRow.className = "sb-title session-title-row";

  var titleText = document.createElement("span");
  titleText.textContent = "Sessions";
  titleRow.appendChild(titleText);

  var newBtn = document.createElement("button");
  newBtn.className = "session-new-btn";
  newBtn.title = "New session";
  newBtn.textContent = "+";
  newBtn.addEventListener("click", createNewSession);
  titleRow.appendChild(newBtn);

  section.appendChild(titleRow);

  var container = document.createElement("div");
  container.id = "sessionList";
  container.className = "session-list-container";
  section.appendChild(container);

  state.sessionListContainer = container;

  /* Insert at the top of sidebar */
  sidebar.insertBefore(section, sidebar.firstChild);

  /* Extract project_id from URL params or project dir */
  var params = new URLSearchParams(location.search);
  state.currentProjectId = params.get("project_id") || null;

  /* Initial load */
  refreshSessionList();
}

/* -- Inject inline styles for session panel -- */

(function injectSessionStyles() {
  var style = document.createElement("style");
  style.textContent = [
    ".session-section { border-bottom: 1px solid #30363d; }",
    ".session-title-row { display:flex; justify-content:space-between; align-items:center; }",
    ".session-new-btn { background:#21262d; border:1px solid #30363d; color:#58a6ff; border-radius:4px; width:24px; height:24px; cursor:pointer; font-size:16px; line-height:1; display:flex; align-items:center; justify-content:center; }",
    ".session-new-btn:hover { background:#30363d; }",
    ".session-list-container { max-height: 280px; overflow-y: auto; }",
    ".session-list { display:flex; flex-direction:column; gap:2px; }",
    ".session-empty { color:#8b949e; font-size:12px; padding:8px 0; }",
    ".session-item { display:flex; align-items:center; justify-content:space-between; padding:6px 8px; border-radius:6px; cursor:pointer; transition:background .15s; }",
    ".session-item:hover { background:#161b22; }",
    ".session-item.session-active { background:#1c2128; border-left:2px solid #58a6ff; }",
    ".session-info { flex:1; min-width:0; overflow:hidden; }",
    ".session-title { color:#c9d1d9; font-size:13px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }",
    ".session-meta { color:#8b949e; font-size:11px; margin-top:2px; }",
    ".session-actions { display:flex; gap:4px; opacity:0; transition:opacity .15s; flex-shrink:0; margin-left:6px; }",
    ".session-item:hover .session-actions { opacity:1; }",
    ".session-btn { background:none; border:none; color:#8b949e; cursor:pointer; font-size:13px; padding:2px 4px; border-radius:3px; }",
    ".session-btn:hover { color:#c9d1d9; background:#30363d; }",
    ".session-del-btn:hover { color:#ff4444; }"
  ].join("\n");
  document.head.appendChild(style);
})();

/* -- Exports -- */
H.fetchSessions = fetchSessions;
H.createSession = createSession;
H.fetchMessages = fetchMessages;
H.updateSessionTitle = updateTitle;
H.deleteSession = deleteSession;
H.forkSession = forkSession;
H.renderSessionList = renderSessionList;
H.switchSession = switchSession;
H.forkAndSwitch = forkAndSwitch;
H.confirmDeleteSession = confirmDeleteSession;
H.maybeAutoTitle = maybeAutoTitle;
H.handleSessionWelcome = handleSessionWelcome;
H.refreshSessionList = refreshSessionList;
H.createNewSession = createNewSession;
H.initSessionManager = initSessionManager;

})(window.HexChat);
