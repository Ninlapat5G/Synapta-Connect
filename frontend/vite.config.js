import tailwind from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig(({ mode }) => ({
  base: mode === 'desktop' ? './' : '/synaptaconnect/',
  plugins: [tailwind()],
  oxc: { jsx: { runtime: 'automatic' } },
  build: {
    assetsInlineLimit: 0,
    outDir: mode === 'desktop' ? 'dist-desktop' : 'dist',
    rolldownOptions: {
      onLog(level, log, handler) {
        if (log.code === 'MODULE_LEVEL_DIRECTIVE' && /node_modules[\\/](framer-motion|lucide-react)[\\/]/.test(log.id || log.message)) return;
        handler(level, log);
      },
    },
  },
}));
