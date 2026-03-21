import { Component, Show, createSignal } from 'solid-js';

interface ToolCallCardProps {
  toolName: string;
  input?: string;
  result?: string;
  isError?: boolean;
}

const ToolCallCard: Component<ToolCallCardProps> = (props) => {
  const [expanded, setExpanded] = createSignal(false);

  return (
    <div class="my-2 rounded-lg border border-gray-700 bg-gray-900/60 overflow-hidden">
      <button
        class="flex w-full items-center gap-2 px-3 py-2 text-left text-xs transition-colors hover:bg-gray-800/50"
        classList={{ 'border-b border-gray-700': expanded() }}
        onClick={() => setExpanded(!expanded())}
      >
        <span class="text-purple-400">&#9881;</span>
        <span class="font-mono font-medium text-gray-200">{props.toolName}</span>
        <Show when={props.isError}>
          <span class="rounded bg-red-900/40 px-1.5 py-0.5 text-[9px] text-red-300">
            error
          </span>
        </Show>
        <span
          class="ml-auto text-gray-500 transition-transform"
          classList={{ 'rotate-90': expanded() }}
        >
          &#9654;
        </span>
      </button>

      <Show when={expanded()}>
        <div class="space-y-2 px-3 py-2 text-xs">
          <Show when={props.input}>
            <div>
              <div class="mb-1 text-[10px] font-medium uppercase tracking-wider text-gray-500">
                Input
              </div>
              <pre class="whitespace-pre-wrap break-words rounded bg-gray-950/50 p-2 text-gray-400 font-mono">
                {props.input}
              </pre>
            </div>
          </Show>
          <Show when={props.result}>
            <div>
              <div class="mb-1 text-[10px] font-medium uppercase tracking-wider text-gray-500">
                {props.isError ? 'Error' : 'Result'}
              </div>
              <pre
                class="whitespace-pre-wrap break-words rounded bg-gray-950/50 p-2 font-mono"
                classList={{
                  'text-gray-400': !props.isError,
                  'text-red-300': !!props.isError,
                }}
              >
                {props.result}
              </pre>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
};

export default ToolCallCard;
