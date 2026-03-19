/* Message dispatch + connection/agent handlers */
(function(H) {
"use strict";
var state = H.state;
var dom = H.dom;

function handleMessage(msg) {
  switch (msg.type) {
    case "connected":          handleConnected(msg);   break;
    case "stream_chunk":       H.handleStreamChunk(msg); break;
    case "tool_call":          H.handleToolCall(msg);    break;
    case "tool_result":        H.handleToolResult(msg);  break;
    case "token_update":       handleTokenUpdate(msg); break;
    case "agent_status":       handleAgentStatus(msg); break;
    case "agent_connected":    handleAgentConnected(msg); break;
    case "agent_disconnected": handleAgentDisconnected(msg); break;
    case "chat_message":       H.addAssistantMessage(msg.content || ""); break;
    case "swarm_created":      handleHexFloEvent(msg); break;
    case "task_updated":       handleHexFloEvent(msg); break;
    case "agent_spawned":      handleHexFloEvent(msg); break;
    case "agent_terminated":   handleHexFloEvent(msg); break;
  }
}

function handleConnected(msg) {
  if (msg.llmBridge) {
    dom.modelBadge.textContent = "LLM bridge";
    dom.modelBadge.style.background = "rgba(63,185,80,.15)";
    dom.modelBadge.style.color = "#3fb950";
  }
}

function handleAgentConnected(msg) {
  dom.agentStatus.textContent = "connected";
  dom.agentStatus.className = "status-pill active";
  if (msg.agentId && msg.agentName) {
    state.connectedAgents[msg.agentId] = {
      name: msg.agentName, status: "idle", uptime: 0, lastSeen: Date.now()
    };
  }
  H.updateModelBadge();
  if (msg.projectDir) dom.projectDir.textContent = msg.projectDir;
}

function handleAgentDisconnected(msg) {
  if (state.streaming) H.endStream();
  if (msg.agentId) delete state.connectedAgents[msg.agentId];
  var remaining = Object.keys(state.connectedAgents).length;
  dom.agentStatus.textContent = remaining > 0 ? "connected" : "disconnected";
  dom.agentStatus.className = "status-pill " + (remaining > 0 ? "active" : "idle");
  H.updateModelBadge();
}

function handleTokenUpdate(msg) {
  state.totalInput = msg.total_input !== undefined ? msg.total_input : state.totalInput;
  state.totalOutput = msg.total_output !== undefined ? msg.total_output : state.totalOutput;
  var total = state.totalInput + state.totalOutput;
  var pct = Math.min(total / state.tokenBudget, 1);
  updateGauge(pct);
  dom.tokIn.textContent = H.fmt(msg.input_tokens || 0);
  dom.tokOut.textContent = H.fmt(msg.output_tokens || 0);
  dom.tokTotalIn.textContent = H.fmt(state.totalInput);
  dom.tokTotalOut.textContent = H.fmt(state.totalOutput);
}

function updateGauge(pct) {
  var circ = 314.16;
  dom.gaugeFill.style.strokeDashoffset = circ * (1 - pct);
  dom.gaugePct.textContent = Math.round(pct * 100) + "%";
  dom.gaugeFill.style.stroke = pct < 0.6 ? "#3fb950" : pct < 0.85 ? "#e3b341" : "#ff4444";
}

function handleAgentStatus(msg) {
  var s = msg.status || "idle";
  dom.agentStatus.textContent = s;
  dom.agentStatus.className = "status-pill " + (s === "idle" ? "idle" : "active");
  if (msg.detail) dom.projectDir.textContent = msg.detail;
  if (msg.model) dom.modelBadge.textContent = msg.model;
  if (msg.agent_name) {
    var agents = state.connectedAgents;
    var found = false;
    for (var id in agents) {
      if (agents[id].name === msg.agent_name) {
        agents[id].status = s;
        if (msg.uptime_secs !== undefined) agents[id].uptime = msg.uptime_secs;
        agents[id].lastSeen = Date.now();
        found = true;
        break;
      }
    }
    if (!found) {
      state.connectedAgents["auto_" + msg.agent_name] = {
        name: msg.agent_name, status: s, uptime: msg.uptime_secs || 0, lastSeen: Date.now()
      };
      H.updateModelBadge();
    }
  }
  if (s === "idle" && state.streaming) H.endStream();
}

function handleHexFloEvent(msg) {
  var text = "";
  switch (msg.type) {
    case "swarm_created":
      text = "\u2B21 Swarm created: " + (msg.name || msg.id || "new swarm");
      break;
    case "task_updated":
      text = "\u25C9 Task " + (msg.task_id || "?") + " \u2192 " + (msg.status || "updated");
      break;
    case "agent_spawned":
      var a = msg.agent || {};
      text = "\u25B6 Agent spawned: " + (a.name || a.agent_name || a.id || "new agent");
      break;
    case "agent_terminated":
      text = "\u25A0 Agent terminated: " + (msg.agent_id || "?");
      break;
  }
  if (text) H.addSystemMessage(text);
  H.fetchHexFlo();
}

H.handleMessage = handleMessage;

})(window.HexChat);
