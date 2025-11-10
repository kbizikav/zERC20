declare module 'poseidon-lite' {
  export default function poseidon(inputs: readonly (bigint | number)[]): bigint;
}
