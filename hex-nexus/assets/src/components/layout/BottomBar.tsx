import { Component, createSignal } from 'solid-js';

const BottomBar: Component = () => {
  const [value, setValue] = createSignal('');

  const handleSubmit = () => {
    const text = value().trim();
    if (!text) return;

    if (text.startsWith('/')) {
      console.log('[BottomBar] slash command:', text);
    } else {
      console.log('[BottomBar] message:', text);
    }
    setValue('');
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  return (
    <div class="flex items-center gap-3 border-t border-gray-800 bg-gray-900 px-4 py-2">
      <span class="text-gray-300 text-sm select-none">&gt;</span>
      <input
        type="text"
        class="flex-1 bg-transparent text-sm text-gray-100 placeholder-gray-600 outline-none"
        placeholder="Type a message or / for commands..."
        value={value()}
        onInput={(e) => setValue(e.currentTarget.value)}
        onKeyDown={handleKeyDown}
      />
      <span class="shrink-0 rounded bg-gray-800 px-2 py-0.5 text-[10px] font-medium text-gray-100">
        Session
      </span>
    </div>
  );
};

export default BottomBar;
