// hex-nexus/assets/src/components/views/AttentionFeed.tsx
// This component is a single-column attention feed for the hex dashboard MissionControl landing.

import { createSignal, createMemo, For } from 'solid-js';
import { AttentionItem } from './types'; // Assuming types are defined in a separate file

interface AttentionFeedProps {
  items: AttentionItem[];
}

const AttentionFeed = (props: AttentionFeedProps) => {
  const sortedItems = createMemo(() => {
    return props.items.sort((a, b) => {
      if (a.priority !== b.priority) {
        return a.priority - b.priority;
      }
      return b.age_seconds - a.age_seconds;
    });
  });

  const renderPriorityPill = (priority: number) => {
    switch (priority) {
      case 0:
        return <span class="bg-red-500 text-white px-2 py-1 rounded">High</span>;
      case 1:
        return <span class="bg-amber-500 text-white px-2 py-1 rounded">Medium</span>;
      case 2:
        return <span class="bg-blue-500 text-white px-2 py-1 rounded">Low</span>;
      default:
        return null;
    }
  };

  return (
    <div class="space-y-4">
      <For each={sortedItems()}>{item => (
        <div class="bg-white shadow-md p-4 flex flex-col space-y-2">
          {renderPriorityPill(item.priority)}
          <span class="font-semibold">{item.kind}</span>
          <h3 class="text-xl font-bold">{item.title}</h3>
          <p>{item.subtitle}</p>
          <div class="flex items-center space-x-2">
            <span class="text-gray-500">{item.age_seconds} seconds ago</span>
            {item.action_url && (
              <a href={item.action_url} target="_blank" class="text-blue-500 underline">Inspect</a>
            )}
            {item.cli_repro && (
              <button
                onClick={() => {
                  navigator.clipboard.writeText(item.cli_repro);
                }}
                class="bg-gray-200 text-gray-800 px-2 py-1 rounded"
              >
                CLI
              </button>
            )}
          </div>
        </div>
      )}</For>
    </div>
  );
};

export default AttentionFeed;
