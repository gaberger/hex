/* Utility functions for HexChat */
(function(H) {
"use strict";

function esc(s) {
  var d = document.createElement("div");
  d.textContent = s;
  return d.innerHTML;
}

function truncate(s, n) { return s.length > n ? s.slice(0, n) + "..." : s; }

function fmt(n) { return n >= 1e6 ? (n/1e6).toFixed(1)+"M" : n >= 1e3 ? (n/1e3).toFixed(1)+"k" : String(n); }

function fmtUptime(secs) {
  if (secs < 60) return secs + "s";
  if (secs < 3600) return Math.floor(secs / 60) + "m " + (secs % 60) + "s";
  return Math.floor(secs / 3600) + "h " + Math.floor((secs % 3600) / 60) + "m";
}

var agentColors = [
  "#00d4aa","#3b82f6","#f472b6","#a78bfa","#fbbf24",
  "#34d399","#f87171","#60a5fa","#c084fc","#fb923c"
];

function agentColor(name) {
  if (!name) return agentColors[0];
  var h = 0;
  for (var i = 0; i < name.length; i++) h = ((h << 5) - h + name.charCodeAt(i)) | 0;
  return agentColors[Math.abs(h) % agentColors.length];
}

function makeAgentBadge(name) {
  var span = document.createElement("span");
  span.className = "agent-badge";
  var c = agentColor(name);
  span.style.background = c + "22";
  span.style.color = c;
  span.textContent = name;
  return span;
}

H.esc = esc;
H.truncate = truncate;
H.fmt = fmt;
H.fmtUptime = fmtUptime;
H.agentColor = agentColor;
H.makeAgentBadge = makeAgentBadge;

})(window.HexChat);
