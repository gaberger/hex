import { defineConfig } from 'vite'
import solidPlugin from 'vite-plugin-solid'

// Tailwind + autoprefixer are configured via postcss.config.js, so no inline
// css.postcss block is needed here (the previous one pointed at a repo-root
// relative tailwind path that doesn't resolve from this dir).
export default defineConfig({
  plugins: [solidPlugin()],
  server: {
    port: 5173,
  },
})

// ADR-2026-05-19-0721
