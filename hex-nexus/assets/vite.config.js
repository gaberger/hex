import { resolve } from 'path';
import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [
    solid(),
    tailwindcss(),
  ],
  server: {
    host: '0.0.0.0', // expose to LAN (matches `hex nexus start --bind 0.0.0.0`)
    port: 5174,
    strictPort: true,
    hmr: {
      host: '192.168.30.162',
      port: 5174,
    },
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:5555',
        changeOrigin: true,
      },
      '/ws': {
        target: 'ws://127.0.0.1:5555',
        ws: true,
      },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
      },
    },
  },
});
