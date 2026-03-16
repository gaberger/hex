#!/usr/bin/env node
/**
 * Push dependency graph to hex-hub (port 5555).
 * Standalone script — no MCP needed.
 */
const http = require('http');
const path = require('path');

const HUB = { hostname: '127.0.0.1', port: 5555 };
const ROOT = path.resolve(__dirname, '..');

function post(urlPath, data) {
  return new Promise((resolve, reject) => {
    const body = JSON.stringify(data);
    const req = http.request({
      ...HUB, path: urlPath, method: 'POST',
      headers: { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(body) },
    }, (res) => {
      let d = '';
      res.on('data', c => d += c);
      res.on('end', () => {
        try { resolve(JSON.parse(d || '{}')); }
        catch { resolve({ raw: d }); }
      });
    });
    req.on('error', reject);
    req.write(body);
    req.end();
  });
}

function classifyLayer(fp) {
  if (fp.includes('/core/domain/')) return 'domain';
  if (fp.includes('/core/ports/')) return 'port';
  if (fp.includes('/core/usecases/')) return 'usecase';
  if (fp.includes('/adapters/primary/')) return 'primary-adapter';
  if (fp.includes('/adapters/secondary/')) return 'secondary-adapter';
  return 'other';
}

async function main() {
  // 1. Register project
  const reg = await post('/api/register', { name: 'hex-intf', root: ROOT });
  const pid = reg.projectId || reg.project_id || reg.id;
  console.log('Registered project:', pid);

  // 2. Build graph via createAppContext
  const { createAppContext } = require(path.join(ROOT, 'dist', 'index.js'));
  const ctx = await createAppContext(ROOT);
  const edges = await ctx.archAnalyzer.buildDependencyGraph(ROOT);
  const nodeSet = new Set();
  for (const e of edges) { nodeSet.add(e.from); nodeSet.add(e.to); }

  const nodes = Array.from(nodeSet).map(id => ({ id, layer: classifyLayer(id) }));
  const graphEdges = edges.map(e => ({ from: e.from, to: e.to, names: e.names }));
  console.log(`Graph: ${nodes.length} nodes, ${graphEdges.length} edges`);

  // Layer breakdown
  const counts = {};
  nodes.forEach(n => { counts[n.layer] = (counts[n.layer] || 0) + 1; });
  console.log('Layers:', counts);

  // 3. Push graph
  const pushResult = await post(`/api/${pid}/push`, { type: 'graph', data: { nodes, edges: graphEdges } });
  console.log('Push result:', pushResult);
}

main().catch(e => { console.error(e); process.exit(1); });
