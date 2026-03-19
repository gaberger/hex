/* RL Insights polling */
(function(H) {
"use strict";
var dom = H.dom;

function fetchRLStats() {
  fetch("/api/rl/stats").then(function(r) { return r.json(); }).then(function(d) {
    if (d.action) dom.rlAction.textContent = d.action;
    if (d.confidence != null) dom.rlConf.textContent = "confidence: " + Math.round(d.confidence * 100) + "%";
  }).catch(function() {});
}

setInterval(fetchRLStats, 15000);
fetchRLStats();

})(window.HexChat);
