/**
 * Safe UUID v4 generator.
 *
 * `crypto.randomUUID()` only exists in a **secure context** (HTTPS or
 * `localhost`). When the dashboard is served over plain HTTP on a LAN IP
 * (e.g. `http://192.168.x.x:5555`), `crypto.randomUUID` is `undefined` and
 * call sites throw "crypto.randomUUID is not a function" — which broke swarm
 * creation, fleet nodes, task creation, and chat ids.
 *
 * This prefers `randomUUID`, falls back to `crypto.getRandomValues` (available
 * in insecure contexts too), and finally to `Math.random` so it never throws.
 */
export function uuid(): string {
  const c: Crypto | undefined = (globalThis as any).crypto;
  if (c?.randomUUID) {
    return c.randomUUID();
  }
  if (c?.getRandomValues) {
    const b = new Uint8Array(16);
    c.getRandomValues(b);
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 10
    const h = Array.from(b, (x) => x.toString(16).padStart(2, "0"));
    return `${h[0]}${h[1]}${h[2]}${h[3]}-${h[4]}${h[5]}-${h[6]}${h[7]}-${h[8]}${h[9]}-${h[10]}${h[11]}${h[12]}${h[13]}${h[14]}${h[15]}`;
  }
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (ch) => {
    const r = (Math.random() * 16) | 0;
    return (ch === "x" ? r : (r & 0x3) | 0x8).toString(16);
  });
}
