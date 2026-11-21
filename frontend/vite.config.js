import tailwind from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  base: '/synaptaconnect/',
  plugins: [tailwind()],
  oxc: { jsx: { runtime: 'automatic' } },
  build: { assetsInlineLimit: 0 },
});
