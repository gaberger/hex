/**
 * tauri-bridge.js — Thin bridge between hex-hub's vanilla JS frontend and Tauri native APIs.
 *
 * Exposes `window.hexNative` with feature-detected native capabilities.
 * Falls back gracefully when running in a regular browser (headless mode).
 */
(function () {
  'use strict';

  const isTauri = Boolean(window.__TAURI_INTERNALS__);

  if (!isTauri) {
    // Running in browser — expose a no-op stub so callers don't crash
    window.hexNative = {
      available: false,
      openProject: async function () { return null; },
      notify: function () {},
      getHubStatus: async function () { return null; },
      getVersion: async function () { return null; },
      spawnAgent: async function () { return null; },
      killAgent: async function () { return null; },
      listAgents: async function () { return []; },
      onAgentEvent: function () { return function () {}; },
    };
    return;
  }

  // Tauri is available — wire up real native calls
  const invoke = window.__TAURI_INTERNALS__.invoke;

  window.hexNative = {
    available: true,

    /** Open a native folder picker and return the selected path (or null). */
    openProject: async function () {
      try {
        return await invoke('open_project');
      } catch (e) {
        console.error('[hexNative] openProject failed:', e);
        return null;
      }
    },

    /** Show a native OS notification. */
    notify: function (title, body) {
      invoke('plugin:notification|notify', { title: title, body: body }).catch(function (e) {
        console.warn('[hexNative] notification failed:', e);
      });
    },

    /** Get hub status from the Rust backend. */
    getHubStatus: async function () {
      try {
        return await invoke('get_hub_status');
      } catch (e) {
        console.error('[hexNative] getHubStatus failed:', e);
        return null;
      }
    },

    /** Get version string. */
    getVersion: async function () {
      try {
        return await invoke('get_hub_version');
      } catch (e) {
        console.error('[hexNative] getVersion failed:', e);
        return null;
      }
    },

    /** Spawn an agent with the given definition in the specified project directory. */
    spawnAgent: async function (definition, projectPath) {
      try {
        return await invoke('spawn_agent', { definition: definition, projectPath: projectPath });
      } catch (e) {
        console.error('[hexNative] spawnAgent failed:', e);
        return null;
      }
    },

    /** Kill a running agent by its ID. */
    killAgent: async function (agentId) {
      try {
        return await invoke('kill_agent', { agentId: agentId });
      } catch (e) {
        console.error('[hexNative] killAgent failed:', e);
        return null;
      }
    },

    /** List all currently registered agents. */
    listAgents: async function () {
      try {
        return await invoke('list_agents');
      } catch (e) {
        console.error('[hexNative] listAgents failed:', e);
        return [];
      }
    },

    /** Subscribe to agent lifecycle events. Returns an unlisten function. */
    onAgentEvent: function (callback) {
      if (window.__TAURI_INTERNALS__.event && window.__TAURI_INTERNALS__.event.listen) {
        var unlisten = window.__TAURI_INTERNALS__.event.listen('agent-event', function (event) {
          try {
            callback(event.payload);
          } catch (e) {
            console.error('[hexNative] onAgentEvent callback error:', e);
          }
        });
        // listen() returns a Promise<UnlistenFn>
        return function () {
          unlisten.then(function (fn) { fn(); });
        };
      }
      console.warn('[hexNative] event.listen not available — agent events will not fire');
      return function () {};
    },
  };

  console.log('[hexNative] Tauri bridge initialized — native capabilities available');
})();
