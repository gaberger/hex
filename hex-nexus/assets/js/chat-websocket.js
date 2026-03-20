/* WebSocket connection management */
(function(H) {
"use strict";
var state = H.state;
var dom = H.dom;

function getAuthToken() {
  var h = location.hash.slice(1);
  if (h) return h;
  var params = new URLSearchParams(location.search);
  var t = params.get("token");
  if (t) { location.hash = t; return t; }
  t = prompt("Enter hub auth token:", "");
  if (t) { location.hash = t; return t; }
  return null;
}

function connect() {
  state.authToken = state.authToken || getAuthToken();
  if (!state.authToken) return;
  var proto = location.protocol === "https:" ? "wss:" : "ws:";
  var host = location.host || "localhost:5555";
  var url = proto + "//" + host + "/ws/chat?token=" + encodeURIComponent(state.authToken);
  var ws = new WebSocket(url);
  state.ws = ws;
  ws.onopen = function() {
    state.connected = true; state.reconnectDelay = 1000;
    dom.connDot.classList.remove("disconnected");
    dom.connLabel.textContent = "connected";
  };
  ws.onclose = function() {
    state.connected = false;
    dom.connDot.classList.add("disconnected");
    dom.connLabel.textContent = "disconnected";
    scheduleReconnect();
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
}

function scheduleReconnect() {
  if (state.reconnectTimer) return;
  state.reconnectTimer = setTimeout(function() {
    state.reconnectTimer = null;
    state.reconnectDelay = Math.min(state.reconnectDelay * 2, 30000);
    connect();
  }, state.reconnectDelay);
}

function wsSend(content) {
  if (!state.ws || state.ws.readyState !== 1) return;
  var payload = { type: "chat_message", content: content };
  // Attach selected model if available
  if (H.selectedModel) { payload.model = H.selectedModel; }
  var atMatch = content.match(/^@(\S+)\s+([\s\S]*)$/);
  if (atMatch) {
    var targetName = atMatch[1];
    for (var id in state.connectedAgents) {
      if (state.connectedAgents[id].name === targetName) {
        payload.agent_id = id.replace(/^auto_/, "");
        payload.content = atMatch[2];
        break;
      }
    }
  }
  state.ws.send(JSON.stringify(payload));
}

H.connect = connect;
H.wsSend = wsSend;

})(window.HexChat);
