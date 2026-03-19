/* Stream chunk handling */
(function(H) {
"use strict";
var state = H.state;
var dom = H.dom;

function handleStreamChunk(msg) {
  var incomingAgent = msg.agent_name || null;
  if (!state.streaming || !state.currentAssistantEl || (incomingAgent && incomingAgent !== state.currentAgentName)) {
    if (state.streaming) endStream();
    state.streaming = true;
    state.currentAgentName = incomingAgent;
    state.currentAssistantEl = H.createMsgEl("assistant");
    if (incomingAgent) {
      var roleEl = state.currentAssistantEl.querySelector(".msg-role");
      roleEl.textContent = "";
      roleEl.appendChild(H.makeAgentBadge(incomingAgent));
    }
  }
  var body = state.currentAssistantEl.querySelector(".msg-body");
  body.dataset.raw = (body.dataset.raw || "") + (msg.text || "");
  var span = document.createElement("span");
  span.textContent = msg.text || "";
  body.appendChild(span);
  body.classList.add("streaming-cursor");
  H.scrollToBottom();
}

function endStream() {
  if (state.currentAssistantEl) {
    var body = state.currentAssistantEl.querySelector(".msg-body");
    body.classList.remove("streaming-cursor");
    if (body.dataset.raw) {
      H.setBodyContent(body, body.dataset.raw);
    }
    state.turnCount++;
    dom.turnCount.textContent = state.turnCount;
  }
  state.streaming = false;
  state.currentAssistantEl = null;
  state.currentAgentName = null;
}

H.handleStreamChunk = handleStreamChunk;
H.endStream = endStream;

})(window.HexChat);
