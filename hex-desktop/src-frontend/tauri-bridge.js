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
  };

  console.log('[hexNative] Tauri bridge initialized — native capabilities available');
})();
