/* HexFlo polling — swarms + agents sidebar panels */
(function(H) {
"use strict";
var state = H.state;
var dom = H.dom;

function normalizeTaskStatus(s) {
  if (!s) return "pending";
  s = s.toLowerCase();
  if (s === "completed" || s === "done") return "done";
  if (s === "in_progress" || s === "in-progress" || s === "running" || s === "active") return "in-progress";
  if (s === "failed" || s === "error") return "failed";
  return "pending";
}

function fetchHexFlo() {
  fetch("/api/swarms/active").then(function(r) { return r.json(); }).then(function(swarms) {
    state.hexfloSwarms = Array.isArray(swarms) ? swarms : [];
    renderSwarms();
  }).catch(function() {});

  fetch("/api/agents").then(function(r) { return r.json(); }).then(function(agents) {
    state.hexfloAgentList = Array.isArray(agents) ? agents : [];
    renderAgentList();
  }).catch(function() {});
}

function renderSwarms() {
  var swarms = state.hexfloSwarms;
  if (swarms.length === 0) {
    dom.hexfloSwarms.textContent = "";
    var empty = document.createElement("span");
    empty.className = "hexflo-empty";
    empty.textContent = "No active swarms";
    dom.hexfloSwarms.appendChild(empty);
    return;
  }
  dom.hexfloSwarms.textContent = "";
  for (var i = 0; i < swarms.length; i++) {
    var s = swarms[i];
    var tasks = s.tasks || [];
    var done = 0;
    for (var j = 0; j < tasks.length; j++) {
      if (tasks[j].status === "done" || tasks[j].status === "completed") done++;
    }
    var total = tasks.length || 1;
    var pct = Math.round((done / total) * 100);

    var card = document.createElement("div");
    card.className = "swarm-card";

    var hdr = document.createElement("div");
    hdr.className = "swarm-header";
    var hex = document.createElement("span");
    hex.className = "swarm-hex";
    hex.textContent = "\u2B21";
    var name = document.createElement("span");
    name.className = "swarm-name";
    name.textContent = s.name || s.id || "swarm";
    var topo = document.createElement("span");
    topo.className = "swarm-topo";
    topo.textContent = s.topology || "mesh";
    hdr.appendChild(hex);
    hdr.appendChild(name);
    hdr.appendChild(topo);

    var prog = document.createElement("div");
    prog.className = "swarm-progress";
    var fill = document.createElement("div");
    fill.className = "swarm-progress-fill";
    fill.style.width = pct + "%";
    prog.appendChild(fill);

    var stats = document.createElement("div");
    stats.className = "swarm-stats";
    var left = document.createElement("span");
    left.textContent = done + "/" + tasks.length + " tasks";
    var right = document.createElement("span");
    right.textContent = pct + "%";
    stats.appendChild(left);
    stats.appendChild(right);

    card.appendChild(hdr);
    card.appendChild(prog);
    card.appendChild(stats);

    if (tasks.length > 0) {
      var tl = document.createElement("div");
      tl.className = "task-list";
      for (var k = 0; k < tasks.length; k++) {
        var t = tasks[k];
        var row = document.createElement("div");
        row.className = "task-row";
        var dot = document.createElement("span");
        dot.className = "task-dot " + normalizeTaskStatus(t.status);
        var title = document.createElement("span");
        title.className = "task-title";
        title.textContent = t.title || t.id || "task";
        var st = document.createElement("span");
        st.className = "task-status " + normalizeTaskStatus(t.status);
        st.textContent = normalizeTaskStatus(t.status);
        row.appendChild(dot);
        row.appendChild(title);
        row.appendChild(st);
        tl.appendChild(row);
      }
      card.appendChild(tl);
    }

    dom.hexfloSwarms.appendChild(card);
  }
}

function renderAgentList() {
  var agents = state.hexfloAgentList;
  if (agents.length === 0) {
    dom.hexfloAgents.textContent = "";
    var empty = document.createElement("span");
    empty.className = "hexflo-empty";
    empty.textContent = "No agents";
    dom.hexfloAgents.appendChild(empty);
    return;
  }
  dom.hexfloAgents.textContent = "";
  for (var i = 0; i < agents.length; i++) {
    var a = agents[i];
    var card = document.createElement("div");
    card.className = "agent-card";

    var top = document.createElement("div");
    top.className = "agent-card-top";
    var badge = H.makeAgentBadge(a.name || a.agent_name || a.id || "agent");
    badge.className = "agent-card-name";
    var c = H.agentColor(a.name || a.agent_name || a.id || "agent");
    badge.style.color = c;
    var statusEl = document.createElement("span");
    var agStatus = (a.status || a.health || "idle").toLowerCase();
    var statusClass = (agStatus === "active" || agStatus === "running" || agStatus === "healthy") ? "active" : "idle";
    statusEl.className = "agent-card-status " + statusClass;
    statusEl.textContent = agStatus;
    top.appendChild(badge);
    top.appendChild(statusEl);
    card.appendChild(top);

    if (a.uptime_secs !== undefined || a.spawned_at) {
      var uptime = document.createElement("div");
      uptime.className = "agent-card-uptime";
      if (a.uptime_secs !== undefined) {
        uptime.textContent = H.fmtUptime(a.uptime_secs);
      } else if (a.spawned_at) {
        var elapsed = Math.floor((Date.now() - new Date(a.spawned_at).getTime()) / 1000);
        uptime.textContent = H.fmtUptime(Math.max(0, elapsed));
      }
      card.appendChild(uptime);
    }

    if (a.current_task || a.assigned_task) {
      var taskEl = document.createElement("div");
      taskEl.className = "agent-card-task";
      taskEl.textContent = a.current_task || a.assigned_task;
      card.appendChild(taskEl);
    }

    dom.hexfloAgents.appendChild(card);
  }
}

H.fetchHexFlo = fetchHexFlo;

setInterval(fetchHexFlo, 10000);
fetchHexFlo();

})(window.HexChat);
