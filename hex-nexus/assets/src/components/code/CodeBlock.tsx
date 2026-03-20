/**
 * CodeBlock.tsx — Syntax-highlighted code display.
 *
 * Uses CSS class-based highlighting with a minimal token regex.
 * For full Shiki integration, replace highlightCode() with Shiki's
 * codeToHtml() when the dependency is added.
 */
import { Component, createMemo } from "solid-js";

interface CodeBlockProps {
  code: string;
  language?: string;
  filename?: string;
  showLineNumbers?: boolean;
}

// Minimal keyword-based highlighting (Shiki replacement)
const KEYWORD_RE = /\b(const|let|var|function|return|if|else|for|while|import|export|from|class|interface|type|async|await|try|catch|throw|new|this|true|false|null|undefined)\b/g;
const STRING_RE = /("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`)/g;
const COMMENT_RE = /(\/\/.*$|\/\*[\s\S]*?\*\/)/gm;
const NUMBER_RE = /\b(\d+\.?\d*)\b/g;

function highlightCode(code: string): string {
  // Order matters: comments first (they can contain strings/keywords)
  let result = code
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");

  result = result.replace(COMMENT_RE, '<span class="text-gray-300">$1</span>');
  result = result.replace(STRING_RE, '<span class="text-green-400">$1</span>');
  result = result.replace(KEYWORD_RE, '<span class="text-purple-400">$1</span>');
  result = result.replace(NUMBER_RE, '<span class="text-cyan-400">$1</span>');

  return result;
}

const CodeBlock: Component<CodeBlockProps> = (props) => {
  const highlighted = createMemo(() => highlightCode(props.code));
  const lines = createMemo(() => props.code.split("\n"));
  const showNumbers = () => props.showLineNumbers ?? true;

  return (
    <div class="rounded-lg border border-gray-800 bg-gray-900/80 overflow-hidden">
      {/* Header */}
      {props.filename && (
        <div class="flex items-center gap-2 border-b border-gray-800 px-3 py-1.5">
          <svg class="h-3.5 w-3.5 text-gray-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
            <polyline points="14 2 14 8 20 8" />
          </svg>
          <span class="text-[11px] font-mono text-gray-300">{props.filename}</span>
          {props.language && (
            <span class="ml-auto rounded bg-gray-800 px-1.5 py-0.5 text-[9px] text-gray-300">
              {props.language}
            </span>
          )}
        </div>
      )}

      {/* Code */}
      <div class="overflow-auto">
        <pre class="p-3 text-xs leading-5 font-mono">
          <code innerHTML={highlighted()} />
        </pre>
      </div>
    </div>
  );
};

export default CodeBlock;
