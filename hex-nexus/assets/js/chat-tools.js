/* Tool call / tool result rendering */
(function(H) {
"use strict";
var state = H.state;

function handleToolCall(msg) {
  if (!state.currentAssistantEl) {
    state.streaming = true;
    state.currentAgentName = msg.agent_name || null;
    state.currentAssistantEl = H.createMsgEl("assistant");
    if (msg.agent_name) {
      var roleEl = state.currentAssistantEl.querySelector(".msg-role");
      roleEl.textContent = "";
      roleEl.appendChild(H.makeAgentBadge(msg.agent_name));
    }
  }
  var body = state.currentAssistantEl.querySelector(".msg-body");
  body.classList.remove("streaming-cursor");

  var card = document.createElement("div");
  card.className = "tool-card";
  card.dataset.toolId = msg.tool_use_id || msg.tool_name + "_" + Date.now();

  var header = document.createElement("div");
  header.className = "tool-header";
  header.addEventListener("click", function() { card.classList.toggle("expanded"); });
  var icon = document.createElement("span");
  icon.className = "tool-icon";
  icon.textContent = "\u2699";
  var nameEl = document.createElement("span");
  nameEl.className = "tool-name";
  nameEl.textContent = msg.tool_name;
  var chevron = document.createElement("span");
  chevron.className = "tool-chevron";
  chevron.textContent = "\u25B6";
  header.appendChild(icon);
  header.appendChild(nameEl);
  header.appendChild(chevron);

  var detail = document.createElement("div");
  detail.className = "tool-detail";
  var inputSection = document.createElement("div");
  inputSection.className = "tool-section";
  var inputLabel = document.createElement("div");
  inputLabel.className = "tool-label";
  inputLabel.textContent = "Input";
  inputSection.appendChild(inputLabel);
  inputSection.appendChild(document.createTextNode(
    H.truncate(JSON.stringify(msg.tool_input, null, 2), 200)
  ));

  var resultSection = document.createElement("div");
  resultSection.className = "tool-section tool-result-section";
  var resultLabel = document.createElement("div");
  resultLabel.className = "tool-label";
  resultLabel.textContent = "Result";
  var runningSpan = document.createElement("span");
  runningSpan.style.color = "var(--text3)";
  runningSpan.textContent = "running...";
  resultSection.appendChild(resultLabel);
  resultSection.appendChild(runningSpan);

  detail.appendChild(inputSection);
  detail.appendChild(resultSection);
  card.appendChild(header);
  card.appendChild(detail);
  body.appendChild(card);
  H.scrollToBottom();
}

function handleToolResult(msg) {
  if (!state.currentAssistantEl) return;
  var card = null;
  if (msg.tool_use_id) {
    card = state.currentAssistantEl.querySelector('.tool-card[data-tool-id="' + CSS.escape(msg.tool_use_id) + '"]');
  }
  if (!card) {
    var cards = state.currentAssistantEl.querySelectorAll('.tool-card[data-tool-id]');
    for (var i = cards.length - 1; i >= 0; i--) {
      if (cards[i].dataset.toolId.indexOf(msg.tool_name) === 0) { card = cards[i]; break; }
    }
  }
  if (!card) return;
  if (msg.is_error) {
    card.querySelector(".tool-header").classList.add("error");
  }
  var resSection = card.querySelector(".tool-result-section");
  resSection.textContent = "";
  var label = document.createElement("div");
  label.className = "tool-label";
  label.textContent = msg.is_error ? "Result (error)" : "Result";
  resSection.appendChild(label);
  var content = typeof msg.content === "string" ? msg.content : JSON.stringify(msg.content, null, 2);
  resSection.appendChild(document.createTextNode(H.truncate(content, 800)));

  var body = state.currentAssistantEl.querySelector(".msg-body");
  body.classList.add("streaming-cursor");
  H.scrollToBottom();
}

H.handleToolCall = handleToolCall;
H.handleToolResult = handleToolResult;

})(window.HexChat);
