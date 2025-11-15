import { createRequire } from 'node:module';
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { nodePolyfills } from 'vite-plugin-node-polyfills';
import path from 'node:path';

const require = createRequire(import.meta.url);
const nobleHashesDir = path.dirname(require.resolve('@noble/hashes'));

const nobleHashesResolver = {
  name: 'noble-hashes-resolver',
  resolveId(source: string) {
    if (!source.startsWith('@noble/hashes/')) {
      return null;
    }

    const subpath = source.slice('@noble/hashes/'.length);
    const target = subpath.endsWith('.js') ? subpath : `${subpath}.js`;
    return path.join(nobleHashesDir, target);
  },
};

export default defineConfig({
  plugins: [react(), nodePolyfills(), nobleHashesResolver],
  resolve: {
    alias: [
      { find: '@', replacement: path.resolve(__dirname, 'src') },
      { find: '@app', replacement: path.resolve(__dirname, 'src/app') },
      { find: '@components', replacement: path.resolve(__dirname, 'src/components') },
      { find: '@features', replacement: path.resolve(__dirname, 'src/features') },
      { find: '@services', replacement: path.resolve(__dirname, 'src/services') },
      { find: '@config', replacement: path.resolve(__dirname, 'src/config') },
      { find: '@utils', replacement: path.resolve(__dirname, 'src/utils') },
      {
        find: 'vite-plugin-node-polyfills/shims/global',
        replacement: path.resolve(__dirname, 'node_modules/vite-plugin-node-polyfills/shims/global'),
      },
      {
        find: 'vite-plugin-node-polyfills/shims/process',
        replacement: path.resolve(__dirname, 'node_modules/vite-plugin-node-polyfills/shims/process'),
      },
    ],
  },
  assetsInclude: ['**/*.wasm']
});
