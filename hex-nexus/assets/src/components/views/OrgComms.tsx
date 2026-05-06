/**
 * OrgComms.tsx — Organization Communication Flow Visualizer
 *
 * Shows real-time message routing through the org hierarchy:
 * - CEO (you) sends message
 * - Routes to appropriate executive
 * - Executive delegates to leads
 * - Leads coordinate ICs
 * - Results aggregate back up
 */

import { Component, For, Show, createSignal, onMount, createMemo } from "solid-js";
import { restClient } from "../../services/rest-client";

interface Message {
  id: string;
  from: string;
  to: string;
  content: string;
  timestamp: string;
  status: "sent" | "routing" | "delegated" | "completed";
}

interface CommunicationFlow {
  messages: Message[];
  active_agents: string[];
}

const OrgComms: Component = () => {
  const [flow, setFlow] = createSignal<CommunicationFlow | null>(null);
  const [selectedMessage, setSelectedMessage] = createSignal<Message | null>(null);
  const [loading, setLoading] = createSignal(true);

  onMount(async () => {
    // TODO: Connect to real-time message flow from SpacetimeDB
    // For now, show mock structure
    setFlow({
      messages: [
        {
          id: "1",
          from: "ceo",
          to: "cto",
          content: "What's engineering health?",
          timestamp: new Date().toISOString(),
          status: "routing"
        }
      ],
      active_agents: ["cto", "engineering-lead"]
    });
    setLoading(false);
  });

  const MessagePath: Component<{ message: Message }> = (props) => {
    return (
      <div class="border border-gray-700 rounded-lg p-4 mb-4 bg-gray-900">
        <div class="flex items-center gap-4 mb-2">
          <div class="text-cyan-400 font-semibold">{props.message.from}</div>
          <div class="text-gray-500">→</div>
          <div class="text-purple-400 font-semibold">{props.message.to}</div>
          <div class={`ml-auto px-2 py-1 rounded text-xs ${
            props.message.status === "completed" ? "bg-green-900 text-green-300" :
            props.message.status === "routing" ? "bg-yellow-900 text-yellow-300" :
            "bg-blue-900 text-blue-300"
          }`}>
            {props.message.status}
          </div>
        </div>
        <div class="text-gray-300 text-sm">{props.message.content}</div>
        <div class="text-gray-600 text-xs mt-2">
          {new Date(props.message.timestamp).toLocaleString()}
        </div>
      </div>
    );
  };

  return (
    <div class="flex flex-col bg-gray-950 h-screen">
      <div class="p-6 border-b border-gray-800">
        <h1 class="text-2xl font-bold text-white mb-2">Organization Communication Flow</h1>
        <p class="text-gray-400">
          Real-time message routing through the hierarchy
        </p>
      </div>

      <Show when={loading()}>
        <div class="text-center py-12 text-gray-400">Loading communication flow...</div>
      </Show>

      <Show when={!loading() && flow()}>
        <div class="flex flex-1 gap-6 overflow-hidden p-6">
          {/* Message Flow */}
          <div class="flex-1 overflow-y-auto">
            <h2 class="text-lg font-semibold text-white mb-4">Active Messages</h2>
            <For each={flow()!.messages}>
              {(message) => <MessagePath message={message} />}
            </For>

            {/* Architecture Diagram */}
            <div class="mt-8">
              <h2 class="text-lg font-semibold text-white mb-4">Communication Architecture</h2>
              <div class="bg-gray-900 border border-gray-700 rounded-lg p-6">
                <pre class="text-gray-300 text-sm font-mono whitespace-pre">
{`┌─────────────────────────────────────────────────────┐
│ YOU (CEO)                                           │
│ • Give directives                                   │
│ • Ask for status                                    │
│ • Approve/reject decisions                          │
└──────────────┬──────────────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────────────┐
│ Brain UI (hex-nexus dashboard)                      │
│ • Parse CEO message                                 │
│ • Route to appropriate executive                    │
│ • /api/chat/send → SpacetimeDB                      │
└──────────────┬──────────────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────────────┐
│ SpacetimeDB (coordination layer)                    │
│ • agent-registry: track who's available             │
│ • hexflo-coordination: task assignment              │
│ • message routing by reports_to hierarchy           │
└──────────────┬──────────────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────────────┐
│ Executive Layer (CTO, CPO, COO, CISO, CVO)          │
│ • Receive CEO directive                             │
│ • Delegate to leads                                 │
│ • Aggregate reports from leads                      │
└──────────────┬──────────────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────────────┐
│ Lead Layer (engineering-lead, product-lead, etc)    │
│ • Break down tasks                                  │
│ • Assign to IC agents                               │
│ • Monitor progress                                  │
│ • Report status up                                  │
└──────────────┬──────────────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────────────┐
│ IC Layer (hex-coder, hex-tester, etc)              │
│ • Execute work in git worktrees                     │
│ • Run compile/lint/test feedback loops              │
│ • Report completion to lead                         │
└─────────────────────────────────────────────────────┘

Status reports bubble back up the same chain.
Each level aggregates information before reporting.`}
                </pre>
              </div>
            </div>

            {/* Example Flow */}
            <div class="mt-8">
              <h2 class="text-lg font-semibold text-white mb-4">Example: "CTO, what's engineering health?"</h2>
              <div class="space-y-2">
                <div class="flex items-center gap-4 p-3 bg-gray-900 rounded border-l-4 border-cyan-500">
                  <div class="w-8 h-8 bg-cyan-600 rounded-full flex items-center justify-center text-white font-bold">1</div>
                  <div>
                    <div class="text-white font-semibold">CEO → Brain UI</div>
                    <div class="text-gray-400 text-sm">Message: "CTO, what's engineering health?"</div>
                  </div>
                </div>

                <div class="flex items-center gap-4 p-3 bg-gray-900 rounded border-l-4 border-purple-500">
                  <div class="w-8 h-8 bg-purple-600 rounded-full flex items-center justify-center text-white font-bold">2</div>
                  <div>
                    <div class="text-white font-semibold">Brain → SpacetimeDB</div>
                    <div class="text-gray-400 text-sm">Route to: cto (tier: executive)</div>
                  </div>
                </div>

                <div class="flex items-center gap-4 p-3 bg-gray-900 rounded border-l-4 border-blue-500">
                  <div class="w-8 h-8 bg-blue-600 rounded-full flex items-center justify-center text-white font-bold">3</div>
                  <div>
                    <div class="text-white font-semibold">CTO Agent Spawns</div>
                    <div class="text-gray-400 text-sm">Reads: engineering-lead status, validation-judge reports</div>
                  </div>
                </div>

                <div class="flex items-center gap-4 p-3 bg-gray-900 rounded border-l-4 border-green-500">
                  <div class="w-8 h-8 bg-green-600 rounded-full flex items-center justify-center text-white font-bold">4</div>
                  <div>
                    <div class="text-white font-semibold">Engineering-lead Queries ICs</div>
                    <div class="text-gray-400 text-sm">Checks: hex-coder task status, test pass rate, build health</div>
                  </div>
                </div>

                <div class="flex items-center gap-4 p-3 bg-gray-900 rounded border-l-4 border-yellow-500">
                  <div class="w-8 h-8 bg-yellow-600 rounded-full flex items-center justify-center text-white font-bold">5</div>
                  <div>
                    <div class="text-white font-semibold">ICs Report Status</div>
                    <div class="text-gray-400 text-sm">hex-coder: 3 tasks in progress, hex-tester: 95% pass rate</div>
                  </div>
                </div>

                <div class="flex items-center gap-4 p-3 bg-gray-900 rounded border-l-4 border-orange-500">
                  <div class="w-8 h-8 bg-orange-600 rounded-full flex items-center justify-center text-white font-bold">6</div>
                  <div>
                    <div class="text-white font-semibold">Engineering-lead Aggregates</div>
                    <div class="text-gray-400 text-sm">Summary: 7 ICs active, 3 features in progress, 2 blockers</div>
                  </div>
                </div>

                <div class="flex items-center gap-4 p-3 bg-gray-900 rounded border-l-4 border-red-500">
                  <div class="w-8 h-8 bg-red-600 rounded-full flex items-center justify-center text-white font-bold">7</div>
                  <div>
                    <div class="text-white font-semibold">CTO Reports to CEO</div>
                    <div class="text-gray-400 text-sm">Health: Good. Velocity on track. 2 blockers need attention.</div>
                  </div>
                </div>
              </div>
            </div>
          </div>

          {/* Active Agents */}
          <div class="w-80 shrink-0 overflow-y-auto border-l border-gray-800 pl-6">
            <h2 class="text-lg font-semibold text-white mb-4">Active Agents</h2>
            <For each={flow()!.active_agents}>
              {(agent) => (
                <div class="bg-gray-900 border border-gray-700 rounded p-3 mb-2">
                  <div class="text-cyan-400 font-semibold">{agent}</div>
                  <div class="text-gray-500 text-xs mt-1">Processing...</div>
                </div>
              )}
            </For>

            <div class="mt-8">
              <h3 class="text-sm font-semibold text-gray-400 mb-2">Next Steps</h3>
              <div class="text-gray-500 text-xs">
                <p class="mb-2">To fully enable this flow:</p>
                <ul class="list-disc list-inside space-y-1">
                  <li>Parse @mentions in Brain chat (e.g., "@cto status?")</li>
                  <li>Route to agent-registry by persona name</li>
                  <li>Stream responses back through org hierarchy</li>
                  <li>Real-time SpacetimeDB subscription for active tasks</li>
                </ul>
              </div>
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default OrgComms;
