/**
 * Shared helper for reading and mutating .claude/settings.json via the REST file API.
 */

const SETTINGS_PATH = '.claude/settings.json';

/**
 * Read the current settings.json, apply an updater function, then write it back.
 * Returns true on success, false on failure.
 */
export async function updateSettingsJson(updater: (settings: any) => void): Promise<boolean> {
  try {
    const res = await fetch(`/api/files?path=${encodeURIComponent(SETTINGS_PATH)}`);
    if (!res.ok) return false;
    const data = await res.json();
    const settings = JSON.parse(data.content || '{}');
    updater(settings);
    const saveRes = await fetch('/api/files', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: SETTINGS_PATH, content: JSON.stringify(settings, null, 2) }),
    });
    return saveRes.ok;
  } catch {
    return false;
  }
}
