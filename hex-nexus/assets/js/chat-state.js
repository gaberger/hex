/* HexChat namespace — shared state and DOM refs for all chat modules */
"use strict";
window.HexChat = {
  state: {
    ws: null, connected: false, reconnectDelay: 1000, reconnectTimer: null,
    turnCount: 0, streaming: false, currentAssistantEl: null, authToken: null,
    totalInput: 0, totalOutput: 0, tokenBudget: 200000,
    connectedAgents: {},
    currentAgentName: null,
    hexfloSwarms: [],
    hexfloAgentList: []
  },
  dom: {}
};

(function(H) {
  var ids = [
    "messages", "input", "sendBtn", "connDot", "connLabel",
    "gaugeFill", "gaugePct", "tokIn", "tokOut", "tokTotalIn", "tokTotalOut",
    "agentStatus", "turnCount", "projectDir", "rlAction", "rlConf",
    "modelBadge", "sidebar", "sidebarToggle", "hexfloSwarms", "hexfloAgents",
    "filePanel", "filePanelToggle", "fpBack", "fpClose", "fpBreadcrumb", "fpBody"
  ];
  for (var i = 0; i < ids.length; i++) {
    H.dom[ids[i]] = document.getElementById(ids[i]);
  }
})(window.HexChat);
