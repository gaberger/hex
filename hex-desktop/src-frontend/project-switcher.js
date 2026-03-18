/**
 * project-switcher.js — Recent-projects sidebar component for hex-desktop.
 *
 * Maintains a list of recent projects in localStorage, renders a compact
 * sidebar panel, and delegates project-open to hexNative.openProject().
 *
 * Usage:
 *   <div id="projectSwitcher"></div>
 *   <script src="project-switcher.js"></script>
 *   <script>HexProjectSwitcher.mount(document.getElementById('projectSwitcher'));</script>
 */
(function () {
  'use strict';

  var STORAGE_KEY = 'hex-recent-projects';
  var MAX_PROJECTS = 10;

  /* ── State ── */
  var recentProjects = []; // [{path: string}]
  var activeProject = null; // path string or null
  var rootEl = null;

  /* ── Persistence ── */
  function loadProjects() {
    try {
      var raw = localStorage.getItem(STORAGE_KEY);
      if (raw) {
        recentProjects = JSON.parse(raw);
        if (!Array.isArray(recentProjects)) recentProjects = [];
      }
    } catch (e) {
      recentProjects = [];
    }
  }

  function saveProjects() {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(recentProjects));
    } catch (e) {
      console.warn('[project-switcher] Failed to save projects:', e);
    }
  }

  function addProject(path) {
    if (!path) return;
    // Remove duplicate if exists
    recentProjects = recentProjects.filter(function (p) { return p.path !== path; });
    // Prepend
    recentProjects.unshift({ path: path });
    // Enforce FIFO limit
    if (recentProjects.length > MAX_PROJECTS) {
      recentProjects = recentProjects.slice(0, MAX_PROJECTS);
    }
    saveProjects();
  }

  /* ── Helpers ── */
  function basename(p) {
    if (!p) return '(unknown)';
    var parts = p.replace(/[/\\]+$/, '').split(/[/\\]/);
    return parts[parts.length - 1] || p;
  }

  /* ── Notify hub of project switch ── */
  function switchProject(path) {
    activeProject = path;
    // Send project-switch to hub via WebSocket if available
    if (window.hexNative && window.hexNative.available) {
      window.hexNative.openProject().catch(function () {});
    }
    // Also try the REST API used by the dashboard
    fetch('/api/project/switch', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: path })
    }).catch(function () {
      // Fallback — send via any open WebSocket chat connection
      try {
        var ws = document.querySelector('[data-hex-ws]');
        if (ws && ws.__hexWs && ws.__hexWs.readyState === 1) {
          ws.__hexWs.send(JSON.stringify({ type: 'project_switch', path: path }));
        }
      } catch (e) { /* silent */ }
    });
    render();
  }

  /* ── Open Project handler ── */
  async function handleOpenProject() {
    var hexNative = window.hexNative;
    if (hexNative && hexNative.available) {
      var path = await hexNative.openProject();
      if (path) {
        addProject(path);
        switchProject(path);
      }
    } else {
      // Browser fallback — prompt for path
      var path = prompt('Enter project path:');
      if (path && path.trim()) {
        path = path.trim();
        addProject(path);
        switchProject(path);
      }
    }
  }

  /* ── Render ── */
  function render() {
    if (!rootEl) return;
    // Clear
    rootEl.textContent = '';

    // Container
    var container = document.createElement('div');
    container.className = 'hex-ps-container';

    // Header row
    var header = document.createElement('div');
    header.className = 'hex-ps-header';
    var title = document.createElement('span');
    title.className = 'hex-ps-title';
    title.textContent = 'Projects';
    var openBtn = document.createElement('button');
    openBtn.className = 'hex-ps-open-btn';
    openBtn.textContent = '+ Open';
    openBtn.addEventListener('click', handleOpenProject);
    header.appendChild(title);
    header.appendChild(openBtn);
    container.appendChild(header);

    // Project list
    var list = document.createElement('div');
    list.className = 'hex-ps-list';

    if (recentProjects.length === 0) {
      var empty = document.createElement('div');
      empty.className = 'hex-ps-empty';
      empty.textContent = 'No recent projects';
      list.appendChild(empty);
    } else {
      for (var i = 0; i < recentProjects.length; i++) {
        (function (proj) {
          var item = document.createElement('div');
          item.className = 'hex-ps-item';
          if (activeProject === proj.path) {
            item.classList.add('active');
          }

          var dot = document.createElement('span');
          dot.className = 'hex-ps-dot';
          if (activeProject === proj.path) {
            dot.classList.add('hex-ps-dot-active');
          }

          var name = document.createElement('span');
          name.className = 'hex-ps-name';
          name.textContent = basename(proj.path);
          name.title = proj.path;

          var removeBtn = document.createElement('button');
          removeBtn.className = 'hex-ps-remove';
          removeBtn.textContent = '\u00d7';
          removeBtn.title = 'Remove from recent';
          removeBtn.addEventListener('click', function (e) {
            e.stopPropagation();
            recentProjects = recentProjects.filter(function (p) { return p.path !== proj.path; });
            if (activeProject === proj.path) activeProject = null;
            saveProjects();
            render();
          });

          item.appendChild(dot);
          item.appendChild(name);
          item.appendChild(removeBtn);
          item.addEventListener('click', function () {
            switchProject(proj.path);
          });

          list.appendChild(item);
        })(recentProjects[i]);
      }
    }

    container.appendChild(list);
    rootEl.appendChild(container);
  }

  /* ── Inject styles ── */
  function injectStyles() {
    if (document.getElementById('hex-ps-styles')) return;
    var style = document.createElement('style');
    style.id = 'hex-ps-styles';
    style.textContent = [
      '.hex-ps-container{font-family:"SF Mono","Fira Code","Cascadia Code",Consolas,monospace;font-size:.82rem;color:#e0e0e0}',
      '.hex-ps-header{display:flex;align-items:center;justify-content:space-between;margin-bottom:10px}',
      '.hex-ps-title{font-size:.7rem;text-transform:uppercase;letter-spacing:1.5px;color:#6a6a80;font-weight:700}',
      '.hex-ps-open-btn{padding:4px 10px;border:1px solid #252535;border-radius:6px;background:transparent;color:#9e9eb0;cursor:pointer;font-size:.75rem;font-family:inherit;transition:all .15s}',
      '.hex-ps-open-btn:hover{border-color:#00d4aa;color:#00d4aa}',
      '.hex-ps-list{display:flex;flex-direction:column;gap:2px}',
      '.hex-ps-item{display:flex;align-items:center;gap:8px;padding:6px 8px;border-radius:6px;cursor:pointer;transition:background .15s;user-select:none}',
      '.hex-ps-item:hover{background:rgba(255,255,255,.04)}',
      '.hex-ps-item.active{background:rgba(0,212,170,.08)}',
      '.hex-ps-dot{width:7px;height:7px;border-radius:50%;background:#353550;flex-shrink:0;transition:background .2s,box-shadow .2s}',
      '.hex-ps-dot-active{background:#3fb950;box-shadow:0 0 6px rgba(63,185,80,.5)}',
      '.hex-ps-name{flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#e0e0e0;font-size:.82rem}',
      '.hex-ps-remove{background:none;border:none;color:#6a6a80;cursor:pointer;font-size:1rem;padding:0 2px;line-height:1;opacity:0;transition:opacity .15s,color .15s}',
      '.hex-ps-item:hover .hex-ps-remove{opacity:1}',
      '.hex-ps-remove:hover{color:#ff4444}',
      '.hex-ps-empty{color:#6a6a80;font-size:.78rem;padding:8px 0;text-align:center}',
    ].join('\n');
    document.head.appendChild(style);
  }

  /* ── Public API ── */
  window.HexProjectSwitcher = {
    /**
     * Mount the project switcher into a DOM element.
     * @param {HTMLElement} el — container element
     */
    mount: function (el) {
      rootEl = el;
      injectStyles();
      loadProjects();
      render();
    },

    /** Add a project path externally and re-render. */
    addProject: function (path) {
      addProject(path);
      render();
    },

    /** Set the active project externally and re-render. */
    setActive: function (path) {
      activeProject = path;
      render();
    },

    /** Get the currently active project path. */
    getActive: function () {
      return activeProject;
    },

    /** Get all recent project paths. */
    getRecent: function () {
      return recentProjects.map(function (p) { return p.path; });
    },
  };
})();
