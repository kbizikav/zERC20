// SPDX-License-Identifier: BUSL-1.1

declare module 'poseidon-lite' {
  export default function poseidon(inputs: readonly (bigint | number)[]): bigint;
}
