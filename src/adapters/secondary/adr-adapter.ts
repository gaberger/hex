/**
 * ADR Secondary Adapter
 *
 * Parses ADR markdown files from the filesystem and indexes them
 * into AgentDB as searchable patterns. Implements IADRPort.
 *
 * Markdown parsing handles both formats:
 *   ## Status: Accepted
 *   **Status:** Accepted
 *
 * AgentDB calls are best-effort — failures are logged, not thrown.
 */

import type { IFileSystemPort, ADREntry, ADRStatus } from '../../core/ports/index.js';
import type { ISwarmPort } from '../../core/ports/swarm.js';
import type { IADRPort } from '../../core/ports/adr.js';

const ADR_GLOB = 'docs/adrs/ADR-*.md';
const ADR_PATTERN_CATEGORY = 'adr';

/** Known statuses for normalization. */
const VALID_STATUSES: ADRStatus[] = ['proposed', 'accepted', 'deprecated', 'superseded', 'rejected'];

function normalizeStatus(raw: string): ADRStatus {
  const lower = raw.trim().toLowerCase();
  if (VALID_STATUSES.includes(lower as ADRStatus)) return lower as ADRStatus;
  return 'proposed'; // default for unrecognized
}

/** Extract ADR ID from filename, e.g. "docs/adrs/ADR-001-foo.md" → "ADR-001" */
function extractId(filePath: string): string {
  const basename = filePath.split('/').pop() ?? '';
  const match = basename.match(/^(ADR-\d+)/);
  return match ? match[1] : basename.replace('.md', '');
}

/** Parse an ADR markdown string into an ADREntry. */
export function parseADRMarkdown(content: string, filePath: string): ADREntry {
  const lines = content.split('\n');
  let title = '';
  let status: ADRStatus = 'proposed';
  let date = '';
  const sections: string[] = [];
  const linkedFeatures: string[] = [];
  const linkedWorktrees: string[] = [];

  for (const line of lines) {
    // Title: first H1 line
    if (!title && line.startsWith('# ')) {
      title = line.slice(2).trim();
      // Strip ADR ID prefix from title if present (e.g. "ADR-001: Foo" → "Foo")
      title = title.replace(/^ADR-\d+[:\s]+/, '');
    }

    // Status: "## Status: Accepted" or "**Status:** Accepted"
    const statusMatch = line.match(/^##\s*Status:\s*(.+)$/i)
      ?? line.match(/^\*\*Status:\*\*\s*(.+)$/i);
    if (statusMatch) {
      status = normalizeStatus(statusMatch[1]);
    }

    // Date: "## Date: 2026-03-15" or "**Date:** 2026-03-15"
    const dateMatch = line.match(/^##\s*Date:\s*(.+)$/i)
      ?? line.match(/^\*\*Date:\*\*\s*(.+)$/i);
    if (dateMatch) {
      date = dateMatch[1].trim();
    }

    // Section headings (## level)
    if (line.startsWith('## ') && !line.match(/^##\s*(Status|Date):/i)) {
      sections.push(line.slice(3).trim());
    }

    // Feature references: "feat/<name>" or "feature: <name>"
    const featMatch = line.match(/feat\/([a-z0-9-]+)/gi);
    if (featMatch) {
      for (const m of featMatch) {
        const name = m.replace('feat/', '');
        if (!linkedFeatures.includes(name)) linkedFeatures.push(name);
      }
    }

    // Worktree branch references
    const wtMatch = line.match(/worktree[:\s]+([a-z0-9\-/]+)/gi);
    if (wtMatch) {
      for (const m of wtMatch) {
        const branch = m.replace(/worktree[:\s]+/i, '');
        if (!linkedWorktrees.includes(branch)) linkedWorktrees.push(branch);
      }
    }
  }

  return {
    id: extractId(filePath),
    title,
    status,
    date,
    filePath,
    sections,
    linkedFeatures,
    linkedWorktrees,
  };
}

export class ADRAdapter implements IADRPort {
  constructor(
    private readonly fs: IFileSystemPort,
    private readonly swarm: ISwarmPort,
  ) {}

  async scanAll(): Promise<ADREntry[]> {
    const files = await this.fs.glob(ADR_GLOB);
    const entries: ADREntry[] = [];

    for (const filePath of files) {
      try {
        const content = await this.fs.read(filePath);
        entries.push(parseADRMarkdown(content, filePath));
      } catch {
        // Skip unreadable files
      }
    }

    // Sort by ID numerically
    entries.sort((a, b) => {
      const numA = parseInt(a.id.replace('ADR-', ''), 10);
      const numB = parseInt(b.id.replace('ADR-', ''), 10);
      return numA - numB;
    });

    return entries;
  }

  async getById(id: string): Promise<ADREntry | null> {
    const entries = await this.scanAll();
    return entries.find((e) => e.id === id) ?? null;
  }

  async getLastModified(filePath: string): Promise<number> {
    return this.fs.mtime(filePath);
  }

  async indexIntoAgentDB(): Promise<{ indexed: number; errors: number }> {
    const entries = await this.scanAll();
    let indexed = 0;
    let errors = 0;

    for (const entry of entries) {
      try {
        await this.swarm.patternStore({
          name: `${entry.id}: ${entry.title}`,
          category: ADR_PATTERN_CATEGORY,
          content: JSON.stringify({
            id: entry.id,
            title: entry.title,
            status: entry.status,
            date: entry.date,
            sections: entry.sections,
            linkedFeatures: entry.linkedFeatures,
          }),
          confidence: entry.status === 'accepted' ? 0.9 : 0.5,
          tags: [entry.status, ...entry.sections.map((s) => s.toLowerCase())],
        });
        indexed++;
      } catch {
        errors++;
      }
    }

    return { indexed, errors };
  }

  async search(query: string, limit = 10): Promise<ADREntry[]> {
    // Try AgentDB pattern search first
    try {
      const patterns = await this.swarm.patternSearch(query, ADR_PATTERN_CATEGORY, limit);
      if (patterns.length > 0) {
        const ids = patterns.map((p) => {
          try {
            return JSON.parse(p.content).id as string;
          } catch {
            return p.name.match(/^(ADR-\d+)/)?.[1] ?? '';
          }
        }).filter(Boolean);

        const allEntries = await this.scanAll();
        return allEntries.filter((e) => ids.includes(e.id));
      }
    } catch {
      // AgentDB unavailable — fall through to local search
    }

    // Fallback: local text search
    const lowerQuery = query.toLowerCase();
    const allEntries = await this.scanAll();
    return allEntries
      .filter((e) =>
        e.title.toLowerCase().includes(lowerQuery)
        || e.id.toLowerCase().includes(lowerQuery)
        || e.sections.some((s) => s.toLowerCase().includes(lowerQuery)),
      )
      .slice(0, limit);
  }
}
