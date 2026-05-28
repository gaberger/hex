import { defineConfig } from 'vite'
import solidPlugin from 'vite-plugin-solid'
import tailwindcss from 'tailwindcss'

export default defineConfig({
  plugins: [solidPlugin()],
  css: {
    postcss: {
      plugins: [
        tailwindcss('./examples/ebay-clone/frontend/tailwind.config.js'),
      ],
    },
  },
  server: {
    port: 5173,
  },
})

// ADR-2026-05-19-0721