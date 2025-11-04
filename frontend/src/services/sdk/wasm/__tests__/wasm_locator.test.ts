import { describe, expect, it } from 'vitest';

import { configureWasmLocator } from '../index.js';

describe('configureWasmLocator', () => {
  it('sets the override for absolute base URLs', () => {
    const globalMock: { __ZKERC20_WASM_PATH__?: string } = {};

    configureWasmLocator({
      baseUrl: 'https://example.com/app/',
      globalObject: globalMock,
    });

    expect(globalMock.__ZKERC20_WASM_PATH__).toBe(
      'https://example.com/app/wasm/zkerc20_wasm_bg.wasm',
    );
  });

  it('computes absolute override from relative base and origin', () => {
    const globalMock = {
      location: { origin: 'https://example.com' },
    } as { __ZKERC20_WASM_PATH__?: string; location: { origin: string } };

    configureWasmLocator({
      baseUrl: '/nested',
      globalObject: globalMock,
    });

    expect(globalMock.__ZKERC20_WASM_PATH__).toBe(
      'https://example.com/nested/wasm/zkerc20_wasm_bg.wasm',
    );
  });

  it('throws when relative base lacks origin context', () => {
    expect(() =>
      configureWasmLocator({
        baseUrl: '/app',
        globalObject: {},
      }),
    ).toThrowError('location.origin');
  });
});
