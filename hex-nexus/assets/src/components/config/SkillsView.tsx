import { Component, For, createResource, createMemo } from 'solid-js';
import { addToast } from '../../stores/toast';

interface Skill {
  name: string;
  trigger: string;
  desc: string;
  source: string;
}

interface SkillCategory {
  name: string;
  skills: Skill[];
}

const HARDCODED_CATEGORIES: SkillCategory[] = [
  {
    name: "Architecture & Analysis",
    skills: [
      { name: "hex-scaffold", trigger: "/hex-scaffold", desc: "Scaffold a new hexagonal architecture project", source: ".claude/skills/" },
      { name: "hex-generate", trigger: "/hex-generate", desc: "Generate code within an adapter boundary", source: ".claude/skills/" },
      { name: "hex-analyze-arch", trigger: "/hex-analyze-arch", desc: "Check architecture health, find violations, dead exports", source: ".claude/skills/" },
      { name: "hex-analyze-deps", trigger: "/hex-analyze-deps", desc: "Analyze dependencies and recommend tech stack", source: ".claude/skills/" },
      { name: "hex-validate", trigger: "/hex-validate", desc: "Post-build semantic validation with behavioral specs", source: ".claude/skills/" },
    ],
  },
  {
    name: "ADR Management",
    skills: [
      { name: "hex-adr-create", trigger: "/hex-adr-create", desc: "Create a new ADR from TEMPLATE.md with auto-numbering", source: ".claude/skills/" },
      { name: "hex-adr-review", trigger: "/hex-adr-review", desc: "Review code changes against existing ADR decisions", source: ".claude/skills/" },
      { name: "hex-adr-search", trigger: "/hex-adr-search", desc: "Search ADRs by keyword, status, or date range", source: ".claude/skills/" },
      { name: "hex-adr-status", trigger: "/hex-adr-status", desc: "Check ADR lifecycle — find stale, abandoned, or conflicting ADRs", source: ".claude/skills/" },
    ],
  },
  {
    name: "Development Workflow",
    skills: [
      { name: "hex-feature-dev", trigger: "/hex-feature-dev", desc: "Start feature development with hex decomposition and worktree isolation", source: ".claude/skills/" },
      { name: "hex-summarize", trigger: "/hex-summarize", desc: "Generate token-efficient AST summaries of source files", source: ".claude/skills/" },
      { name: "hex-dashboard", trigger: "/hex-dashboard", desc: "Start the hex monitoring dashboard", source: ".claude/skills/" },
    ],
  },
  {
    name: "Git & Review",
    skills: [
      { name: "commit", trigger: "/commit", desc: "Create a git commit with conventional message", source: "built-in" },
      { name: "review-pr", trigger: "/review-pr", desc: "Comprehensive PR review with specialized agents", source: "built-in" },
      { name: "commit-push-pr", trigger: "/commit-push-pr", desc: "Commit, push, and open a pull request", source: "built-in" },
    ],
  },
  {
    name: "Swarm & Orchestration",
    skills: [
      { name: "sparc", trigger: "/sparc", desc: "SPARC methodology — Specification, Pseudocode, Architecture, Refinement, Completion", source: ".claude/skills/" },
      { name: "pair-programming", trigger: "/pair-programming", desc: "AI pair programming with driver/navigator modes", source: "built-in" },
    ],
  },
];

/** Category assignment by name prefix. */
function categorizeSkill(name: string): string {
  if (name.startsWith('hex-adr')) return 'ADR Management';
  if (name.startsWith('hex-analyze') || name.startsWith('hex-scaffold') || name.startsWith('hex-generate') || name.startsWith('hex-validate')) return 'Architecture & Analysis';
  if (name.startsWith('hex-feature') || name.startsWith('hex-summarize') || name.startsWith('hex-dashboard')) return 'Development Workflow';
  if (name.startsWith('sparc') || name.startsWith('pair-')) return 'Swarm & Orchestration';
  return 'Discovered';
}

async function discoverSkills(): Promise<Skill[] | null> {
  try {
    const res = await fetch('/api/files?path=.claude/skills&list=true');
    if (!res.ok) return null;
    const data = await res.json();
    const files: string[] = data.files || [];

    const mdFiles = files.filter((f: string) => f.endsWith('.md'));
    if (mdFiles.length === 0) return null;

    const skills: Skill[] = [];
    for (const file of mdFiles) {
      try {
        const fRes = await fetch(`/api/files?path=${encodeURIComponent('.claude/skills/' + file)}`);
        if (!fRes.ok) continue;
        const fData = await fRes.json();
        const content: string = fData.content || '';

        // Parse YAML frontmatter
        const match = content.match(/^---\n([\s\S]*?)\n---/);
        if (match) {
          const fm = match[1];
          const name = fm.match(/name:\s*(.+)/)?.[1]?.trim() || file.replace('.md', '');
          const trigger = fm.match(/trigger:\s*(.+)/)?.[1]?.trim() || `/${name}`;
          const desc = fm.match(/description:\s*(.+)/)?.[1]?.trim() || '';
          skills.push({ name, trigger, desc, source: '.claude/skills/' });
        } else {
          // No frontmatter — use filename
          const name = file.replace('.md', '');
          skills.push({ name, trigger: `/${name}`, desc: '', source: '.claude/skills/' });
        }
      } catch {
        // skip individual file errors
      }
    }

    return skills.length > 0 ? skills : null;
  } catch {
    return null;
  }
}

const SkillsView: Component = () => {
  const [discoveredSkills] = createResource(discoverSkills);

  const categories = createMemo((): SkillCategory[] => {
    const discovered = discoveredSkills();
    if (!discovered) return HARDCODED_CATEGORIES;

    // Build category map from hardcoded as base
    const catMap = new Map<string, Skill[]>();
    for (const cat of HARDCODED_CATEGORIES) {
      catMap.set(cat.name, [...cat.skills]);
    }

    // Merge discovered skills — replace matching names, add new ones
    const existingNames = new Set(HARDCODED_CATEGORIES.flatMap(c => c.skills.map(s => s.name)));
    for (const skill of discovered) {
      if (existingNames.has(skill.name)) {
        // Update existing skill's description if discovered has one
        for (const [, skills] of catMap) {
          const idx = skills.findIndex(s => s.name === skill.name);
          if (idx >= 0 && skill.desc) {
            skills[idx] = { ...skills[idx], desc: skill.desc };
          }
        }
      } else {
        // New skill — categorize and add
        const catName = categorizeSkill(skill.name);
        if (!catMap.has(catName)) catMap.set(catName, []);
        catMap.get(catName)!.push(skill);
      }
    }

    return Array.from(catMap.entries()).map(([name, skills]) => ({ name, skills }));
  });

  const totalSkills = () => categories().reduce((sum, cat) => sum + cat.skills.length, 0);

  return (
    <div class="flex-1 overflow-auto p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-xl font-bold text-gray-100">Skills</h2>
          <p class="mt-1 text-sm text-gray-400">
            {discoveredSkills.loading ? 'Discovering skills...' : `${totalSkills()} slash commands across ${categories().length} categories`}
          </p>
        </div>
        <button class="rounded-lg border border-gray-700 px-4 py-2 text-sm text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors"
          onClick={() => addToast("info", "Create skills in .claude/skills/ — see /skill-creator for templates")}>
          + New Skill
        </button>
      </div>

      {/* Categorized skills */}
      <div class="space-y-6">
        <For each={categories()}>
          {(category) => (
            <div>
              <h3 class="mb-3 text-xs font-bold uppercase tracking-wider text-gray-500">{category.name}</h3>
              <div class="space-y-1.5">
                <For each={category.skills}>
                  {(skill) => (
                    <div
                      class="flex items-center gap-4 rounded-lg border border-gray-800/50 px-4 py-3 hover:border-gray-700 transition-colors cursor-pointer"
                      style={{ "background-color": "#111827" }}
                    >
                      <span class="text-sm font-bold text-gray-200 min-w-[150px]">{skill.name}</span>
                      <span class="text-xs min-w-[160px]" style={{ "font-family": "'JetBrains Mono', monospace", color: "#22d3ee" }}>
                        {skill.trigger}
                      </span>
                      <span class="text-sm text-gray-400 flex-1">{skill.desc}</span>
                      <span class="shrink-0 rounded-full px-2.5 py-0.5 text-xs font-medium"
                        classList={{
                          "bg-cyan-900/30 text-cyan-400": skill.source !== "built-in",
                          "bg-gray-800 text-gray-500": skill.source === "built-in",
                        }}
                      >
                        {skill.source === "built-in" ? "built-in" : "custom"}
                      </span>
                    </div>
                  )}
                </For>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default SkillsView;
