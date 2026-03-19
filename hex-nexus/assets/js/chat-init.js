/* Initialization — event listeners + connect */
(function(H) {
"use strict";
var dom = H.dom;

/* Input handling */
dom.input.addEventListener("input", function() {
  dom.sendBtn.disabled = !dom.input.value.trim();
  dom.input.style.height = "auto";
  dom.input.style.height = Math.min(dom.input.scrollHeight, 160) + "px";
});

dom.input.addEventListener("keydown", function(e) {
  if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); H.doSend(); }
});

dom.sendBtn.addEventListener("click", H.doSend);

/* Control buttons */
var ctrls = document.querySelectorAll(".ctrl-btn");
for (var i = 0; i < ctrls.length; i++) {
  ctrls[i].addEventListener("click", (function(btn) {
    return function() {
      var cmd = btn.dataset.cmd;
      if (cmd === "/clear") { H.clearMessages(); return; }
      H.wsSend(cmd);
      H.addUserMessage(cmd);
    };
  })(ctrls[i]));
}

/* Sidebar toggle (mobile) */
dom.sidebarToggle.addEventListener("click", function() { dom.sidebar.classList.toggle("open"); });

/* Connect WebSocket */
H.connect();

/* Initialize session manager (ADR-036) */
if (H.initSessionManager) { H.initSessionManager(H.state); }

/* Detect active model from inference endpoints */
(function() {
  var badge = document.getElementById("modelBadge");
  if (!badge) return;
  fetch("/api/inference/endpoints")
    .then(function(r) { return r.json(); })
    .then(function(data) {
      var eps = data.endpoints || [];
      if (eps.length > 0) {
        var model = eps[0].model || "unknown";
        var provider = eps[0].provider || "";
        badge.textContent = model + (provider ? " (" + provider + ")" : "");
      } else if (H.state && H.state.anthropicKey) {
        badge.textContent = "claude-sonnet-4-20250514";
      } else {
        badge.textContent = "no provider";
      }
    })
    .catch(function() { badge.textContent = "offline"; });
})();

/* Update model badge on token_update events */
H.onModelUpdate = function(model) {
  var badge = document.getElementById("modelBadge");
  if (badge && model) badge.textContent = model;
};

})(window.HexChat);
