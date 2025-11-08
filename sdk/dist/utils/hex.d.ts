export type HexLike = string | number | bigint | Uint8Array | {
    toHexString(): string;
} | {
    _hex: string;
};
export declare function normalizeHex(value: HexLike): string;
export declare function hexToBytes(value: string): Uint8Array;
export declare function toBigInt(value: number | string | bigint): bigint;
//# sourceMappingURL=hex.d.ts.map