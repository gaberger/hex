/**
 * DiffViewer.tsx — Inline diff rendering.
 *
 * Renders unified diff output with line-level add/remove coloring.
 * Supports both unified diff format and raw +/- lines.
 */
import { Component, For, createMemo } from "solid-js";

interface DiffLine {
  type: "add" | "remove" | "context" | "header";
  content: string;
  lineNum?: number;
}

function parseDiff(diff: string): DiffLine[] {
  const lines: DiffLine[] = [];
  let addNum = 0;
  let removeNum = 0;

  for (const raw of diff.split("\n")) {
    if (raw.startsWith("@@")) {
      lines.push({ type: "header", content: raw });
      // Parse line numbers from @@ -a,b +c,d @@
      const match = raw.match(/@@ -(\d+)/);
      if (match) removeNum = parseInt(match[1], 10);
      const match2 = raw.match(/\+(\d+)/);
      if (match2) addNum = parseInt(match2[1], 10);
    } else if (raw.startsWith("+")) {
      lines.push({ type: "add", content: raw.slice(1), lineNum: addNum++ });
    } else if (raw.startsWith("-")) {
      lines.push({ type: "remove", content: raw.slice(1), lineNum: removeNum++ });
    } else if (raw.startsWith("diff ") || raw.startsWith("index ") || raw.startsWith("---") || raw.startsWith("+++")) {
      lines.push({ type: "header", content: raw });
    } else {
      lines.push({ type: "context", content: raw.startsWith(" ") ? raw.slice(1) : raw, lineNum: addNum++ });
      removeNum++;
    }
  }

  return lines;
}

const LINE_STYLES = {
  add:     "bg-green-900/20 text-green-300",
  remove:  "bg-red-900/20 text-red-300",
  context: "text-gray-300",
  header:  "bg-blue-900/20 text-blue-400 font-semibold",
};

const GUTTER_STYLES = {
  add:     "text-green-600",
  remove:  "text-red-600",
  context: "text-gray-700",
  header:  "text-blue-600",
};

interface DiffViewerProps {
  diff: string;
  filename?: string;
}

const DiffViewer: Component<DiffViewerProps> = (props) => {
  const lines = createMemo(() => parseDiff(props.diff));

  return (
    <div class="rounded-lg border border-gray-800 bg-gray-900/80 overflow-hidden">
      {/* Header */}
      {props.filename && (
        <div class="flex items-center gap-2 border-b border-gray-800 px-3 py-1.5">
          <svg class="h-3.5 w-3.5 text-gray-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 20h9" /><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z" />
          </svg>
          <span class="text-[11px] font-mono text-gray-300">{props.filename}</span>
          <span class="ml-auto rounded bg-gray-800 px-1.5 py-0.5 text-[9px] text-gray-300">
            diff
          </span>
        </div>
      )}

      {/* Diff lines */}
      <div class="overflow-auto">
        <pre class="text-xs leading-5 font-mono">
          <For each={lines()}>
            {(line) => (
              <div class={`flex ${LINE_STYLES[line.type]}`}>
                {/* Gutter */}
                <span class={`w-12 shrink-0 select-none px-2 text-right ${GUTTER_STYLES[line.type]}`}>
                  {line.type === "add" ? "+" : line.type === "remove" ? "-" : line.type === "header" ? "@@" : " "}
                </span>
                {/* Content */}
                <span class="flex-1 px-2 whitespace-pre">{line.content}</span>
              </div>
            )}
          </For>
        </pre>
      </div>
    </div>
  );
};

export default DiffViewer;
