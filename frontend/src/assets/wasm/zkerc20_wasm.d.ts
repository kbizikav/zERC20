/* tslint:disable */
/* eslint-disable */
export function seed_message(): string;
export function derive_payment_advice(seed_hex: string, payment_advice_id_hex: string, recipient_chain_id: bigint, recipient_address_hex: string): any;
export function derive_invoice_single(seed_hex: string, invoice_id_hex: string, recipient_chain_id: bigint, recipient_address_hex: string): any;
export function derive_invoice_batch(seed_hex: string, invoice_id_hex: string, sub_id: number, recipient_chain_id: bigint, recipient_address_hex: string): any;
export function build_full_burn_address(recipient_chain_id: bigint, recipient_address_hex: string, secret_hex: string, tweak_hex: string): any;
export function decode_full_burn_address(full_burn_address_hex: string): any;
export function general_recipient_fr(chain_id: bigint, recipient_address_hex: string, tweak_hex: string): string;
export function aggregation_root(snapshot_hex: any): string;
export function aggregation_merkle_proof(snapshot_hex: any, index: number): any;
export class SingleWithdrawWasm {
  free(): void;
  [Symbol.dispose](): void;
  constructor(local_pk_bytes: Uint8Array, local_vk_bytes: Uint8Array, global_pk_bytes: Uint8Array, global_vk_bytes: Uint8Array);
  prove(witness: any): any;
}
export class WithdrawNovaWasm {
  free(): void;
  [Symbol.dispose](): void;
  constructor(local_pp_bytes: Uint8Array, local_vp_bytes: Uint8Array, global_pp_bytes: Uint8Array, global_vp_bytes: Uint8Array);
  prove(z0: any, steps: any): any;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly __wbg_withdrawnovawasm_free: (a: number, b: number) => void;
  readonly withdrawnovawasm_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number, number];
  readonly withdrawnovawasm_prove: (a: number, b: any, c: any) => [number, number, number];
  readonly __wbg_singlewithdrawwasm_free: (a: number, b: number) => void;
  readonly singlewithdrawwasm_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number, number];
  readonly singlewithdrawwasm_prove: (a: number, b: any) => [number, number, number];
  readonly seed_message: () => [number, number];
  readonly derive_payment_advice: (a: number, b: number, c: number, d: number, e: bigint, f: number, g: number) => [number, number, number];
  readonly derive_invoice_single: (a: number, b: number, c: number, d: number, e: bigint, f: number, g: number) => [number, number, number];
  readonly derive_invoice_batch: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: number, h: number) => [number, number, number];
  readonly build_full_burn_address: (a: bigint, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number, number];
  readonly decode_full_burn_address: (a: number, b: number) => [number, number, number];
  readonly general_recipient_fr: (a: bigint, b: number, c: number, d: number, e: number) => [number, number, number, number];
  readonly aggregation_root: (a: any) => [number, number, number, number];
  readonly aggregation_merkle_proof: (a: any, b: number) => [number, number, number];
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly __externref_table_alloc: () => number;
  readonly __wbindgen_export_4: WebAssembly.Table;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  readonly __externref_table_dealloc: (a: number) => void;
  readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;
/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
*
* @returns {InitOutput}
*/
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
