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

/* Populate model selector from inference endpoints */
(function() {
  var sel = document.getElementById("modelSelect");
  if (!sel) return;

  H.selectedModel = "";

  fetch("/api/inference/endpoints")
    .then(function(r) { return r.json(); })
    .then(function(data) {
      var eps = data.endpoints || [];
      // Clear placeholder
      while (sel.firstChild) sel.removeChild(sel.firstChild);

      // Collect all models from all providers
      var models = [];
      eps.forEach(function(ep) {
        var provider = ep.provider || "unknown";
        var rawModel = ep.model || "";
        try {
          var parsed = JSON.parse(rawModel);
          if (Array.isArray(parsed)) {
            parsed.forEach(function(m) { models.push({ name: m, provider: provider, url: ep.url || "" }); });
          } else {
            models.push({ name: rawModel, provider: provider, url: ep.url || "" });
          }
        } catch(e) {
          if (rawModel) models.push({ name: rawModel, provider: provider, url: ep.url || "" });
        }
      });

      if (models.length === 0) {
        var opt = document.createElement("option");
        opt.value = "";
        opt.textContent = "no models";
        sel.appendChild(opt);
        return;
      }

      models.forEach(function(m, i) {
        var opt = document.createElement("option");
        opt.value = m.name;
        opt.textContent = m.name + " (" + m.provider + ")";
        if (i === 0) opt.selected = true;
        sel.appendChild(opt);
      });

      H.selectedModel = models[0].name;
      sel.addEventListener("change", function() {
        H.selectedModel = sel.value;
      });
    })
    .catch(function() {
      while (sel.firstChild) sel.removeChild(sel.firstChild);
      var opt = document.createElement("option");
      opt.textContent = "offline";
      sel.appendChild(opt);
    });
})();

/* Update model selector on token_update events */
H.onModelUpdate = function(model) {
  var sel = document.getElementById("modelSelect");
  if (!sel || !model) return;
  // Select matching option if it exists
  for (var i = 0; i < sel.options.length; i++) {
    if (sel.options[i].value === model) { sel.selectedIndex = i; return; }
  }
};

})(window.HexChat);
