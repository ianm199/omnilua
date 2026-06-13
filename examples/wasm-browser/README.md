# omniLua WASM browser example

Static browser playground for the published `omnilua` package.

Run it from the repository root:

```bash
python3 -m http.server 8787
```

Open:

```text
http://127.0.0.1:8787/examples/wasm-browser/
```

The page imports `omnilua@0.1.0` from jsDelivr, loads the packaged
`dist/lua_wasm.wasm`, and runs Lua in the browser with JS-provided stdout,
environment values, stdin, and virtual files.
