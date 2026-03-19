/**
 * chat-dashboard.js — Collapsible left dashboard panel.
 *
 * Shows quick stats, event log, instance status, and token breakdown.
 * Uses safe DOM construction (no innerHTML with dynamic data).
 */
(function (H) {
  'use strict';

  var panel, toggleBtn, eventList;
  var events = [];
  var MAX_EVENTS = 80;

  // ── DOM helpers ────────────────────────────────────────
  function el(tag, className, text) {
    var e = document.createElement(tag);
    if (className) e.className = className;
    if (text != null) e.textContent = String(text);
    return e;
  }

  function svgIcon(paths, cls) {
    var ns = 'http://www.w3.org/2000/svg';
    var svg = document.createElementNS(ns, 'svg');
    svg.setAttribute('viewBox', '0 0 24 24');
    svg.setAttribute('fill', 'none');
    svg.setAttribute('stroke', 'currentColor');
    svg.setAttribute('stroke-width', '2');
    svg.setAttribute('stroke-linecap', 'round');
    svg.setAttribute('stroke-linejoin', 'round');
    paths.forEach(function (d) {
      var p = document.createElementNS(ns, 'path');
      p.setAttribute('d', d);
      svg.appendChild(p);
    });
    var span = el('span', cls || '');
    span.appendChild(svg);
    return span;
  }

  // ── Toggle ─────────────────────────────────────────────
  function toggle() {
    panel.classList.toggle('open');
    document.body.classList.toggle('has-dash-panel');
    toggleBtn.classList.toggle('active');
  }

  // ── Quick Stats ────────────────────────────────────────
  function renderStats(data) {
    var files = document.getElementById('dashFiles');
    var edges = document.getElementById('dashEdges');
    var exports = document.getElementById('dashExports');
    if (files) files.textContent = data.file_count || 0;
    if (edges) edges.textContent = data.edge_count || 0;
    // Total exports = dead + live (dead_export_count is the dead ones)
    var deadExports = data.dead_export_count || 0;
    var totalExports = deadExports > 0 ? deadExports + ' dead' : '0 dead';
    if (exports) exports.textContent = totalExports;
  }

  // ── Event Log ──────────────────────────────────────────
  function addEvent(type, msg) {
    var now = new Date();
    var time = String(now.getHours()).padStart(2, '0') + ':' +
               String(now.getMinutes()).padStart(2, '0') + ':' +
               String(now.getSeconds()).padStart(2, '0');

    events.unshift({ time: time, type: type, msg: msg });
    if (events.length > MAX_EVENTS) events.length = MAX_EVENTS;
    renderEvents();
  }

  function renderEvents() {
    if (!eventList) return;
    eventList.textContent = '';

    if (events.length === 0) {
      eventList.appendChild(el('div', 'dash-events-empty', 'No events yet'));
      return;
    }

    var iconPaths = {
      info: ['M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20z', 'M12 16v-4', 'M12 8h.01'],
      success: ['M22 11.08V12a10 10 0 1 1-5.93-9.14', 'M22 4 12 14.01l-3-3'],
      warn: ['m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z', 'M12 9v4', 'M12 17h.01'],
      error: ['M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20z', 'M15 9l-6 6', 'M9 9l6 6'],
    };

    events.forEach(function (ev) {
      var row = el('div', 'dash-event');
      row.appendChild(el('span', 'dash-event-time', ev.time));
      row.appendChild(svgIcon(iconPaths[ev.type] || iconPaths.info, 'dash-event-icon ' + ev.type));
      row.appendChild(el('span', 'dash-event-msg', ev.msg));
      eventList.appendChild(row);
    });
  }

  // ── Instance Table ─────────────────────────────────────
  function renderInstances(swarms) {
    var container = document.getElementById('dashInstances');
    if (!container) return;
    container.textContent = '';

    if (!swarms || swarms.length === 0) {
      container.appendChild(el('div', 'dash-empty', 'No active instances'));
      return;
    }

    var table = el('table', 'dash-instances');
    var thead = el('thead');
    var headerRow = el('tr');
    ['ID', 'Name', 'Status', 'Tasks'].forEach(function (h) {
      headerRow.appendChild(el('th', '', h));
    });
    thead.appendChild(headerRow);
    table.appendChild(thead);

    var tbody = el('tbody');
    swarms.forEach(function (s) {
      var row = el('tr');
      var idCell = el('td');
      idCell.appendChild(el('span', 'dash-inst-id', (s.id || '').substring(0, 8)));
      row.appendChild(idCell);
      row.appendChild(el('td', '', s.name || '--'));

      var statusCell = el('td');
      var badge = el('span', 'dash-inst-badge ' + (s.status || 'idle'), s.status || 'idle');
      statusCell.appendChild(badge);
      row.appendChild(statusCell);

      var total = (s.tasks || []).length;
      var done = (s.tasks || []).filter(function (t) { return t.status === 'completed'; }).length;
      row.appendChild(el('td', 'dash-inst-time', done + '/' + total));

      tbody.appendChild(row);
    });
    table.appendChild(tbody);
    container.appendChild(table);
  }

  // ── Fetch Data ─────────────────────────────────────────
  async function refresh() {
    try {
      // Fetch analysis
      var projectDir = document.getElementById('projectDir');
      var rootPath = (projectDir && projectDir.textContent !== '--')
        ? projectDir.textContent : '.';

      var res = await fetch('/api/analyze', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ root_path: rootPath }),
      });
      if (res.ok) {
        var data = await res.json();
        renderStats(data);
        addEvent('success', 'Analysis complete — Score: ' + data.health_score + '/100');
      }
    } catch (e) {
      addEvent('error', 'Analysis fetch failed');
    }

    try {
      // Fetch swarms
      var sRes = await fetch('/api/swarms');
      if (sRes.ok) {
        var sData = await sRes.json();
        renderInstances(sData.swarms || sData);
      }
    } catch (e) {
      // Swarm fetch is optional
    }
  }

  // ── Hook into WS events ────────────────────────────────
  // Listen for HexChat WS messages and log relevant ones
  var origOnMessage;
  function hookWebSocket() {
    // Poll until WS is available
    if (!H.state.ws) {
      setTimeout(hookWebSocket, 500);
      return;
    }
    var ws = H.state.ws;
    origOnMessage = ws.onmessage;
    ws.onmessage = function (e) {
      // Call original handler
      if (origOnMessage) origOnMessage.call(ws, e);

      // Log events
      try {
        var msg = JSON.parse(e.data);
        if (msg.type === 'tool_use') {
          addEvent('info', 'Tool: ' + (msg.name || msg.tool || 'unknown'));
        } else if (msg.type === 'error') {
          addEvent('error', msg.message || msg.error || 'Error');
        } else if (msg.type === 'system') {
          addEvent('info', msg.message || msg.text || 'System event');
        } else if (msg.type === 'swarm_update') {
          addEvent('success', 'Swarm: ' + (msg.name || 'updated'));
          renderInstances(msg.swarms || []);
        }
      } catch (_) {}
    };
  }

  // ── Init ───────────────────────────────────────────────
  document.addEventListener('DOMContentLoaded', function () {
    panel = document.getElementById('dashPanel');
    toggleBtn = document.getElementById('dashToggle');
    eventList = document.getElementById('dashEvents');

    if (toggleBtn) toggleBtn.addEventListener('click', toggle);

    var closeBtn = document.getElementById('dashClose');
    if (closeBtn) closeBtn.addEventListener('click', toggle);

    // Initial event
    addEvent('info', 'Dashboard initialized');

    // Fetch data after short delay
    setTimeout(refresh, 2000);

    // Hook into WS for live events
    setTimeout(hookWebSocket, 1500);

    // Auto-refresh every 60s
    setInterval(refresh, 60000);
  });

})(window.HexChat || { state: {}, dom: {} });
