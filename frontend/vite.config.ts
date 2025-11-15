import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { nodePolyfills } from 'vite-plugin-node-polyfills';
import path from 'node:path';

const sdkSource = path.resolve(__dirname, '..', '..', 'zerc20-client-sdk');
const sdkEntry = path.resolve(sdkSource, 'dist', 'index.js');

export default defineConfig({
  plugins: [react(), nodePolyfills()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, 'src'),
      '@app': path.resolve(__dirname, 'src/app'),
      '@components': path.resolve(__dirname, 'src/components'),
      '@features': path.resolve(__dirname, 'src/features'),
      '@services': path.resolve(__dirname, 'src/services'),
      '@config': path.resolve(__dirname, 'src/config'),
      '@utils': path.resolve(__dirname, 'src/utils'),
      '@zerc20/sdk': sdkEntry,
      'vite-plugin-node-polyfills/shims/global': path.resolve(
        __dirname,
        'node_modules/vite-plugin-node-polyfills/shims/global',
      ),
      'vite-plugin-node-polyfills/shims/process': path.resolve(
        __dirname,
        'node_modules/vite-plugin-node-polyfills/shims/process',
      )
    }
  },
  server: {
    fs: {
      allow: [sdkSource, __dirname],
    },
  },
  assetsInclude: ['**/*.wasm']
});
