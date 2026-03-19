/* Core UI helpers: message creation, input handling, controls */
(function(H) {
"use strict";
var state = H.state;
var dom = H.dom;

function clearWelcome() {
  var w = dom.messages.querySelector(".welcome");
  if (w) w.remove();
}

function scrollToBottom() {
  requestAnimationFrame(function() { dom.messages.scrollTop = dom.messages.scrollHeight; });
}

function createMsgEl(role) {
  clearWelcome();
  var wrap = document.createElement("div");
  wrap.className = "msg " + role;
  var roleEl = document.createElement("div");
  roleEl.className = "msg-role";
  roleEl.textContent = role;
  var body = document.createElement("div");
  body.className = "msg-body";
  wrap.appendChild(roleEl);
  wrap.appendChild(body);
  dom.messages.appendChild(wrap);
  scrollToBottom();
  return wrap;
}

function addUserMessage(text) {
  clearWelcome();
  var el = createMsgEl("user");
  H.setBodyContent(el.querySelector(".msg-body"), text);
  state.turnCount++;
  dom.turnCount.textContent = state.turnCount;
  scrollToBottom();
}

function addAssistantMessage(text) {
  H.endStream();
  var el = createMsgEl("assistant");
  H.setBodyContent(el.querySelector(".msg-body"), text);
  scrollToBottom();
}

function addSystemMessage(text) {
  clearWelcome();
  var wrap = document.createElement("div");
  wrap.className = "msg system";
  var body = document.createElement("div");
  body.className = "msg-body";
  body.textContent = text;
  wrap.appendChild(body);
  dom.messages.appendChild(wrap);
  scrollToBottom();
}

function clearMessages() {
  dom.messages.textContent = "";
  var w = document.createElement("div");
  w.className = "welcome";
  var h2 = document.createElement("h2");
  h2.textContent = "hex agent";
  var p = document.createElement("p");
  p.textContent = "Send a message to begin.";
  w.appendChild(h2);
  w.appendChild(p);
  dom.messages.appendChild(w);
  state.turnCount = 0; dom.turnCount.textContent = "0";
  state.streaming = false; state.currentAssistantEl = null;
}

function updateModelBadge() {
  var names = Object.values(state.connectedAgents).map(function(a) { return a.name; });
  if (names.length === 0) return;
  dom.modelBadge.textContent = names.join(", ");
}

function doSend() {
  var text = dom.input.value.trim();
  if (!text) return;
  addUserMessage(text);
  H.wsSend(text);
  dom.input.value = "";
  dom.input.style.height = "auto";
  dom.sendBtn.disabled = true;
}

H.clearWelcome = clearWelcome;
H.scrollToBottom = scrollToBottom;
H.createMsgEl = createMsgEl;
H.addUserMessage = addUserMessage;
H.addAssistantMessage = addAssistantMessage;
H.addSystemMessage = addSystemMessage;
H.clearMessages = clearMessages;
H.updateModelBadge = updateModelBadge;
H.doSend = doSend;

})(window.HexChat);
