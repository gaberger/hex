/**
 * chat-health.js — Architecture Health panel for the chat sidebar.
 *
 * Fetches health data from POST /api/analyze and renders:
 * - SVG health ring with score + grade
 * - Metric summary (violations, dead exports, orphans, circular deps)
 * - Layer breakdown
 * - Issue mini-lists (violations + dead exports)
 *
 * Uses safe DOM construction (no innerHTML with dynamic data).
 */

(function () {
  'use strict';

  const CIRCUMFERENCE = 2 * Math.PI * 34; // r=34 in the SVG

  // ── Grade from score ───────────────────────────────────
  function grade(score) {
    if (score >= 90) return 'A';
    if (score >= 80) return 'B';
    return 'C';
  }

  function gradeColor(score) {
    if (score >= 90) return 'var(--green)';
    if (score >= 80) return 'var(--accent)';
    if (score >= 60) return 'var(--yellow)';
    return 'var(--red)';
  }

  // ── DOM helpers ────────────────────────────────────────
  function el(tag, className, text) {
    const e = document.createElement(tag);
    if (className) e.className = className;
    if (text != null) e.textContent = String(text);
    return e;
  }

  function svgEl(tag, attrs) {
    const e = document.createElementNS('http://www.w3.org/2000/svg', tag);
    for (const [k, v] of Object.entries(attrs || {})) e.setAttribute(k, v);
    return e;
  }

  function metricClass(val) { return val === 0 ? 'ok' : val <= 2 ? 'warn' : 'fail'; }

  // ── SVG Icons (Lucide-style) ───────────────────────────
  function iconSvg(name) {
    const svg = svgEl('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': '2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' });
    const paths = {
      shield: ['M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z'],
      alertTriangle: ['m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z', 'M12 9v4', 'M12 17h.01'],
      trash: ['M3 6h18', 'M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2'],
      file: ['M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z', 'M14 2v6h6'],
      refresh: ['M23 4v6h-6', 'M20.49 15a9 9 0 1 1-2.12-9.36L23 10'],
      layers: ['M12 2 2 7l10 5 10-5-10-5z', 'M2 17l10 5 10-5', 'M2 12l10 5 10-5'],
    };
    (paths[name] || []).forEach(d => {
      const p = svgEl('path', { d });
      svg.appendChild(p);
    });
    return svg;
  }

  function iconSpan(name) {
    const span = el('span', 'health-meta-icon');
    span.appendChild(iconSvg(name));
    return span;
  }

  // ── Build Health Ring SVG ──────────────────────────────
  function buildRing(score) {
    const color = gradeColor(score);
    const offset = CIRCUMFERENCE - (score / 100) * CIRCUMFERENCE;

    const wrap = el('div', 'health-ring');
    const svg = svgEl('svg', { viewBox: '0 0 80 80' });

    const bg = svgEl('circle', { class: 'health-ring-bg', cx: '40', cy: '40', r: '34' });
    const fg = svgEl('circle', {
      class: 'health-ring-fg', cx: '40', cy: '40', r: '34',
      stroke: color, 'stroke-dasharray': String(CIRCUMFERENCE),
      'stroke-dashoffset': String(offset), transform: 'rotate(-90 40 40)',
    });
    svg.appendChild(bg);
    svg.appendChild(fg);
    wrap.appendChild(svg);

    const center = el('div', 'health-center');
    const num = el('span', 'health-score-num', score);
    num.style.color = color;
    const g = el('span', 'health-grade', grade(score));
    center.appendChild(num);
    center.appendChild(g);
    wrap.appendChild(center);

    return wrap;
  }

  // ── Build Metric Row ───────────────────────────────────
  function buildMetricRow(iconName, label, value) {
    const row = el('div', 'health-meta-row');
    row.appendChild(iconSpan(iconName));
    row.appendChild(el('span', 'health-meta-label', label));
    const val = el('span', 'health-meta-value ' + metricClass(value), value);
    row.appendChild(val);
    return row;
  }

  // ── Build Layer Breakdown ──────────────────────────────
  function buildLayers(data) {
    const layerMap = {};
    if (Array.isArray(data.violations)) {
      data.violations.forEach(function (v) {
        var layer = v.from_layer || 'unknown';
        layerMap[layer] = layerMap[layer] || { violations: 0, dead: 0 };
        layerMap[layer].violations++;
      });
    }
    if (Array.isArray(data.dead_exports)) {
      data.dead_exports.forEach(function (d) {
        var file = d.file || '';
        var layer = file.includes('/domain/') ? 'domain'
          : file.includes('/ports/') ? 'ports'
          : file.includes('/usecases/') ? 'usecases'
          : file.includes('/adapters/') ? 'adapters' : 'other';
        layerMap[layer] = layerMap[layer] || { violations: 0, dead: 0 };
        layerMap[layer].dead++;
      });
    }

    const container = el('div', 'health-layers');
    const title = el('div', 'health-layer-title');
    title.appendChild(iconSpan('layers'));
    title.appendChild(document.createTextNode(' Layer Breakdown'));
    container.appendChild(title);

    ['domain', 'ports', 'usecases', 'adapters'].forEach(function (name) {
      const row = el('div', 'health-layer');
      row.appendChild(el('span', 'health-layer-dot ' + name));
      row.appendChild(el('span', 'health-layer-name', name));

      const info = layerMap[name];
      if (!info) {
        row.appendChild(el('span', 'health-layer-stat clean', 'clean'));
      } else {
        const parts = [];
        if (info.violations) parts.push(info.violations + ' violation' + (info.violations > 1 ? 's' : ''));
        if (info.dead) parts.push(info.dead + ' dead');
        row.appendChild(el('span', 'health-layer-stat issues', parts.join(', ')));
      }
      container.appendChild(row);
    });

    return container;
  }

  // ── Build Issue List ───────────────────────────────────
  function buildIssues(data) {
    const container = el('div', 'health-issues');
    var count = 0;

    if (Array.isArray(data.violations)) {
      data.violations.slice(0, 5).forEach(function (v) {
        const item = el('div', 'health-issue violation');
        const fileSpan = el('span', 'issue-file', (v.from_file || '').split('/').pop());
        item.appendChild(fileSpan);
        item.appendChild(document.createTextNode(' ' + (v.rule || '')));
        container.appendChild(item);
        count++;
      });
    }
    if (Array.isArray(data.dead_exports)) {
      data.dead_exports.slice(0, 5).forEach(function (d) {
        const item = el('div', 'health-issue dead');
        const fileSpan = el('span', 'issue-file', (d.file || '').split('/').pop());
        item.appendChild(fileSpan);
        item.appendChild(document.createTextNode(' .' + (d.export_name || '')));
        container.appendChild(item);
        count++;
      });
    }

    return count > 0 ? container : null;
  }

  // ── Build Refresh Button ───────────────────────────────
  function buildRefreshBtn() {
    const btn = el('button', 'health-refresh');
    btn.id = 'healthRefresh';
    btn.title = 'Re-analyze architecture';
    btn.appendChild(iconSvg('refresh'));
    btn.appendChild(document.createTextNode(' Analyze'));
    btn.addEventListener('click', fetchHealth);
    return btn;
  }

  // ── Main Render ────────────────────────────────────────
  function render(data) {
    const panel = document.getElementById('healthPanel');
    if (!panel) return;
    panel.textContent = ''; // Clear safely

    const wrap = el('div', 'health-wrap');

    // Ring + metrics row
    const top = el('div', 'health-ring-wrap');
    top.appendChild(buildRing(data.health_score));

    const meta = el('div', 'health-meta');
    meta.appendChild(buildMetricRow('shield', 'Violations', data.violation_count));
    meta.appendChild(buildMetricRow('trash', 'Dead exports', data.dead_export_count));
    meta.appendChild(buildMetricRow('alertTriangle', 'Circular', data.circular_dep_count));
    meta.appendChild(buildMetricRow('file', 'Orphans', data.orphan_file_count));
    top.appendChild(meta);
    wrap.appendChild(top);

    // Layer breakdown
    wrap.appendChild(buildLayers(data));

    // Issues
    const issues = buildIssues(data);
    if (issues) wrap.appendChild(issues);

    // Refresh button
    wrap.appendChild(buildRefreshBtn());

    panel.appendChild(wrap);
  }

  // ── Fetch ──────────────────────────────────────────────
  async function fetchHealth() {
    const btn = document.getElementById('healthRefresh');
    if (btn) btn.classList.add('loading');

    try {
      const projectDir = document.getElementById('projectDir');
      const rootPath = (projectDir && projectDir.textContent !== '--')
        ? projectDir.textContent
        : '.';

      const res = await fetch('/api/analyze', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ root_path: rootPath }),
      });

      if (!res.ok) throw new Error('HTTP ' + res.status);
      var data = await res.json();
      render(data);
    } catch (err) {
      console.warn('[health] fetch failed:', err.message);
      var panel = document.getElementById('healthPanel');
      if (panel) {
        panel.textContent = '';
        var wrap = el('div', 'health-wrap');
        wrap.appendChild(el('div', '', 'Analysis unavailable'));
        wrap.lastChild.style.cssText = 'color:var(--text3);font-size:.78rem;font-style:italic';
        wrap.appendChild(buildRefreshBtn());
        panel.appendChild(wrap);
      }
    } finally {
      var btn2 = document.getElementById('healthRefresh');
      if (btn2) btn2.classList.remove('loading');
    }
  }

  // Auto-fetch on load
  setTimeout(fetchHealth, 1500);
})();
