import { Component, createEffect, onCleanup } from 'solid-js';
import { marked } from 'marked';

marked.setOptions({
  gfm: true,
  breaks: true,
});

interface MarkdownContentProps {
  content: string;
}

const MarkdownContent: Component<MarkdownContentProps> = (props) => {
  let containerRef: HTMLDivElement | undefined;

  function attachCopyButtons() {
    if (!containerRef) return;
    containerRef.querySelectorAll('pre').forEach((pre) => {
      if (pre.querySelector('.copy-btn')) return;
      const btn = document.createElement('button');
      btn.className =
        'copy-btn absolute top-2 right-2 rounded bg-gray-700 px-2 py-0.5 text-[10px] text-gray-300 opacity-0 transition-opacity hover:bg-gray-600 group-hover:opacity-100';
      btn.textContent = 'copy';
      btn.addEventListener('click', () => {
        const code = pre.querySelector('code');
        if (code) {
          navigator.clipboard.writeText(code.textContent || '');
          btn.textContent = 'copied!';
          setTimeout(() => {
            btn.textContent = 'copy';
          }, 1500);
        }
      });
      pre.style.position = 'relative';
      pre.classList.add('group');
      pre.appendChild(btn);
    });
  }

  const rendered = () => {
    try {
      return marked.parse(props.content) as string;
    } catch {
      return props.content
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/\n/g, '<br>');
    }
  };

  createEffect(() => {
    // Track content changes to re-attach copy buttons
    const _html = rendered();
    // Use queueMicrotask so the DOM has updated after innerHTML is set
    queueMicrotask(attachCopyButtons);
  });

  onCleanup(() => {
    // Remove event listeners by clearing the container reference
    containerRef = undefined;
  });

  return (
    <div
      ref={containerRef}
      class="text-sm text-gray-300 leading-relaxed [&_pre]:bg-gray-900/80 [&_pre]:border [&_pre]:border-gray-700 [&_pre]:rounded-lg [&_pre]:p-3 [&_pre]:my-2 [&_pre]:overflow-x-auto [&_code]:text-cyan-300 [&_code]:bg-gray-800/50 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:text-xs [&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_a]:text-blue-400 [&_h1]:text-lg [&_h1]:font-bold [&_h1]:text-gray-100 [&_h2]:text-base [&_h2]:font-semibold [&_h2]:text-gray-100 [&_h3]:text-sm [&_h3]:font-semibold [&_h3]:text-gray-200 [&_ul]:list-disc [&_ul]:pl-5 [&_ol]:list-decimal [&_ol]:pl-5 [&_li]:my-0.5 [&_p]:my-1.5 [&_blockquote]:border-l-2 [&_blockquote]:border-gray-600 [&_blockquote]:pl-3 [&_blockquote]:text-gray-400 [&_strong]:text-gray-100"
      innerHTML={rendered()}
    />
  );
};

export default MarkdownContent;
