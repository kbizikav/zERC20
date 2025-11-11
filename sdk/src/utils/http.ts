export function ensureFetch(): typeof fetch {
  if (typeof fetch === 'undefined') {
    throw new Error('fetch is not available in the current environment');
  }
  return (input: RequestInfo | URL, init?: RequestInit) => fetch(input, init);
}
