import path from 'node:path';
import tailwindcss from '@tailwindcss/vite';
import { tanstackRouter } from '@tanstack/router-plugin/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// nsp serves the SPA from /ui/ via rust-embed. Setting `base: "/ui/"`
// makes Vite emit asset URLs like `/ui/assets/index-*.js` so deep-link
// reloads keep working.
export default defineConfig({
  base: '/ui/',
  resolve: { alias: { '@': path.resolve(__dirname, './src') } },
  plugins: [
    tanstackRouter({
      target: 'react',
      routesDirectory: 'src/app/routes',
      generatedRouteTree: 'src/app/routeTree.gen.ts',
      autoCodeSplitting: true,
    }),
    react(),
    tailwindcss(),
  ],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
    target: 'es2022',
    chunkSizeWarningLimit: 600,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('node_modules/@base-ui-components/') || id.includes('node_modules/@floating-ui/'))
            return 'base-ui';
          if (id.includes('node_modules/sonner'))
            return 'sonner';
          if (id.includes('node_modules/@tanstack/react-query'))
            return 'query';
          if (id.includes('node_modules/@tanstack/react-router') || id.includes('node_modules/@tanstack/history'))
            return 'router';
          if (id.includes('node_modules/react/') || id.includes('node_modules/react-dom/'))
            return 'react';
        },
      },
    },
  },
  server: {
    host: '0.0.0.0',
    allowedHosts: true,
    port: 5173,
  },
});
