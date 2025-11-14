# WASM

## Build

Use the helper script to build the browser (`web`) and Node.js (`node`) bindings:

```bash
./scripts/build-wasm.sh          # builds both targets
./scripts/build-wasm.sh web      # build only the web bundle
./scripts/build-wasm.sh node     # build only the node bundle
```

Artifacts land in `pkg/<target>/`.

## Test

```bash
wasm-pack test -r --chrome --headless
```
