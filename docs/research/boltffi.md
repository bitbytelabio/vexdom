# BoltFFI as the Rust→TS binding for `ext-core` (spike S2 prep)

Scope: ADR-016 (BoltFFI 0.32 behind spike S2, fallback wasm-bindgen) for the `ext-core` wasm module that ADR-015
puts in the MV3 service worker, offscreen document and extension pages (Chrome) and the MV2 background page
(Firefox). Researched 2026-11-25.

**Sources and pins.** Unless stated otherwise, `boltffi:<path>:<line>` means the BoltFFI repo at commit
`03291fdaae67134a797159f09a5d28b2299abcbb` (2026-11-23,
<https://github.com/boltffi/boltffi/tree/03291fdaae67134a797159f09a5d28b2299abcbb>); the workspace version there is
0.32.1 (`boltffi:Cargo.toml:12`). crates.io lists `boltffi` / `boltffi_cli` 0.32.1, published 2026-10-30
(<https://crates.io/api/v1/crates/boltffi>). npm lists `@boltffi/runtime` 0.32.1, published 2026-10-30
(<https://registry.npmjs.org/@boltffi%2fruntime>). The wasm-bindgen sources cited as `wbg-cli:`, `js-sys:` and `wbg-futures:` are the
crates.io releases `wasm-bindgen-cli-support 0.2.130`, `js-sys 0.3.107` and `wasm-bindgen-futures 0.4.80`.

**What was not possible:**
- `boltffi.dev` did not resolve (`getaddrinfo ENOTFOUND boltffi.dev`, 2026-11-25). The docs site's source is in the repo
  as `boltffi:docs/src/content/docs/*.mdx`, and this file cites that instead. docs.rs built `boltffi` 0.32.1
  (`{"doc_status":true}`, <https://docs.rs/crate/boltffi/0.32.1/status.json>).
- `cargo install boltffi_cli` was refused in this session. So **no glue was generated or measured**. Glue size
  and loader behaviour below come from templates, the runtime package and the upstream tests. The spike
  must confirm them.

## Verdict up front

- BoltFFI's wasm path has **its own C ABI** (`#[no_mangle] extern "C"` plus a wire-buffer format). Its
  TypeScript runtime does not use `eval`, `new Function`, dynamic `import()`, `document` or `window`. The
  runtime instantiates with `WebAssembly.instantiate`. Nothing found makes it unloadable in an MV3 SW or an
  MV2 background page **if** you skip its `web.js`/`bundler.js` loader and call `init(...)` yourself (§2).
- Two structural frictions for vexdom:
  1. **Exported async functions must return `Send` futures** (`boltffi:boltffi_core/src/runtime/future.rs:112,171`).
     `capnp-rpc` objects are `Rc`-based, so a wasm-side capnp client (ADR-014/S1) cannot be awaited directly
     behind a BoltFFI `async fn`. It needs a `spawn_local` + channel bridge, and `spawn_local` drags in wasm-bindgen
     anyway (§4).
  2. rust-anything's wasm bridge produces **wasm-bindgen** output (`rust-anything --help`: "Build the crate's JS
     module (napi-rs `.node`, wasm-bindgen output)"). The per-unit record/diff/gate loop in ADR-015 therefore
     runs on wasm-bindgen whatever `ext-core` ships with.
- Maintenance: there are frequent breaking pre-1.0 releases (0.21→0.32 in 7 months), and one maintainer wrote
  about 64% of the commits (§8). Upstream CI runs wasm in browser *pages* only. There is no worker or extension coverage.

## 1. What `boltffi pack wasm` does

Pipeline, in order (`boltffi:boltffi_cli/src/pack/wasm/mod.rs:39-256`):

1. **Precondition.** It refuses to run unless `targets.wasm.enabled` (`:44-49`) and, for a standalone pack,
   `targets.wasm.npm.package_name` are set (`:50-56`).
2. **Build.** `cargo build` for the wasm triple (`:90-94`, `:272-300`). The triple defaults to
   `wasm32-unknown-unknown` (`boltffi:boltffi_cli/src/config/targets/wasm.rs:130-132`; the artifact path
   is `<target>/wasm32-unknown-unknown/<profile>/<crate>.wasm`, `mod.rs:315-330`). It needs a `cdylib` crate type
   (`boltffi:docs/src/content/docs/getting-started.mdx:42-44`).
3. **wasm-bindgen detection.** If the artifact carries wasm-bindgen metadata (any dependency using
   wasm-bindgen), pack runs the `wasm-bindgen` CLI at the **exact** matching version
   (`mod.rs:119-124`) with `--target bundler --no-typescript`. It then deletes wasm-bindgen's own loader
   (`boltffi:boltffi_cli/src/pack/wasm/bindgen.rs:67-91`). The CLI comes from `targets.wasm.wasm_bindgen_cli`, from
   `BOLTFFI_WASM_BINDGEN`, from `PATH`, or else it is **auto-installed with `cargo install wasm-bindgen-cli
   --version =X --locked`** into a cache (`bindgen.rs:34-65`, `:208-213`). Without metadata, pack just copies the
   wasm (`mod.rs:125-127`).
4. **Strip** custom and debug sections (`mod.rs:129-136`). Debug info goes only when optimization runs
   (`mod.rs:262-270`).
5. **wasm-opt.** It runs on release by default (`boltffi:boltffi_cli/src/config/mod.rs:1050-1056`) at level `-Os`
   (`config/mod.rs:1063`). **A missing `wasm-opt` is an error by default** (`config/mod.rs:1075`;
   `mod.rs:440-455`). Feature flags come from `cargo rustc --print cfg` (`mod.rs:337-422`).
6. **Generate TS.** `boltffi generate typescript` runs, since `--regenerate` defaults to true
   (`boltffi:boltffi_cli/src/cli.rs:143-151`; `mod.rs:149-166`). `--deny-skipped` fails the pack instead of
   silently skipping unsupported declarations (`cli.rs:153-158`).
7. **Transpile.** It shells out to **`tsc` on `PATH`** (`npx tsc` only on Windows). Flags are
   `--target ES2020 --module ES2020 --moduleResolution bundler --declaration`, plus inline source maps,
   which are on by default (`mod.rs:492-570`; `config/mod.rs:1114-1115`). `@boltffi/runtime` must resolve
   from the output's parent directories (`boltffi:runtime/typescript/README.md:42-44`).
8. **Loader entrypoints** `bundler.js` / `web.js` / `node.js`, all three by default
   (`boltffi:boltffi_cli/src/pack/wasm/npm.rs:19-42`; `config/mod.rs:1136-1144`).
9. **`package.json`.** It depends on `@boltffi/runtime` with version **`"*"`** by default
   (`npm.rs:71-74`; `config/mod.rs:1087-1102`).

Output: `<m>_bg.wasm`, `<m>.js` + `.d.ts`, `<m>_imports.js`, the loader entrypoints and `package.json`
(`boltffi:docs/src/content/docs/packaging.mdx:631-663`).

**Docs drift.** The prerequisites list "Node.js 18+ and wasm-pack" (`getting-started.mdx:17`), but the pack code
never calls wasm-pack. It needs `tsc`, optionally `wasm-opt`, and optionally `wasm-bindgen` (above).

## 2. Generated TypeScript loader: ABI, instantiate path, CSP

**Own ABI, not wasm-bindgen.**
- `boltffi` depends only on `boltffi_core` and `boltffi_macros`, with no wasm-bindgen
  (`boltffi:boltffi/Cargo.toml`, `[dependencies]`).
- The wasm exports are `#[unsafe(no_mangle)] pub extern "C"`. One of them is `boltffi_wasm_abi_version()`
  (`boltffi:boltffi_core/src/wasm.rs:193-198`), whose `WASM_ABI_VERSION = 3` (`wasm.rs:4`).
- The JS side checks that ABI version at init and throws on mismatch
  (`boltffi:runtime/typescript/src/module.ts:1753-1758`).
- The import object holds `env.__boltffi_wake`, `env.__boltffi_stream_wake` and the generated callback
  imports (`module.ts:1735-1742`).
- When a dependency uses wasm-bindgen, its `_bg.js` glue is attached to the *same* instance through
  `WasmBindgenModule` (`boltffi:runtime/typescript/src/wasm-bindgen.ts:1-35`;
  `boltffi:boltffi_cli/templates/wasm/imports.ts:1-10`). Only local and inline snippets are allowed. npm imports
  from wasm-bindgen glue are rejected at pack time (`bindgen.rs:113-120`; `runtime/typescript/README.md:46-51`).

**Instantiate path.**
- The core module exports `init(source: BufferSource | Response | WebAssembly.Module)`
  (`boltffi:boltffi_backend/templates/target/typescript/browser.ts:14-37`).
- `instantiateBoltFFI` turns a `Response` into `arrayBuffer()` and then calls `WebAssembly.instantiate`
  (`module.ts:1747-1751`). It **never uses `instantiateStreaming`**, so the MIME type of a
  `chrome-extension://` response does not matter. The cost is losing streaming compile.
- The sync variant uses `new WebAssembly.Module` and `new WebAssembly.Instance` (`module.ts:1769-1792`) and is
  only used by the Node template (`boltffi:boltffi_backend/templates/target/typescript/node.ts:1-9`).

**Loader entrypoints.**
- `bundler.js` and `web.js` render the same code. At import time they run
  `fetch(new URL("./<m>_bg.wasm", import.meta.url))` inside an async IIFE exported as `initialized`
  (`boltffi:boltffi_cli/templates/wasm/loader.js:5-12`). This is a side effect on import and depends on
  `import.meta.url`.
- The core `<m>.js` has no `import.meta`, no top-level await and no fetch (`browser.ts:1-43`).
- `node.ts` uses `node:fs` and `import.meta.url` (`node.ts:4-9`) and must not be bundled for the extension.

**Docs drift.**
- `getting-started.mdx:233-235` shows `import init from './pkg/web.js'; await init();`. But `web.js` has no
  default export, only `export { default as init }` (`loader.js:6-8`).
- `init` requires a `source` argument (`browser.ts:14`).
- Treat the docs as approximate and the templates as truth.

**CSP and global scan.**
- The `@boltffi/runtime@0.32.1` npm tarball (`dist/*.js`, 93,367 bytes) was grepped for
  `eval(`, `new Function`, `Function(`, `import(`, `document.`, `window.`, `importScripts` and
  `instantiateStreaming`. **There were zero hits.** The only WebAssembly APIs found were `instantiate` (×2), `Module` (×2) and
  `Instance` (×1).
- The TS templates contain only `(x as Function)(...)` TypeScript casts, e.g.
  `boltffi:boltffi_backend/templates/target/typescript/class.ts:5`, and no `eval`. A grep of
  `boltffi_backend/templates/target/typescript` and `boltffi_backend/src/target/typescript` found nothing.
- Globals used:
  - `TextEncoder` / `TextDecoder` (`module.ts:363-364`; `wire.ts:1-2`)
  - `queueMicrotask` (`module.ts:94,136`)
  - `FinalizationRegistry`, guarded by `typeof` (`class.ts:1-6`)
  - `BigInt`, `AbortSignal`

  All exist in service workers, extension pages and content scripts. Worker coverage upstream is zero: there are no
  "worker" hits in `runtime/typescript/src`, the TS templates or `examples/platforms/wasm/browser`.

**MV3 / MV2 platform facts that apply to any wasm binding:**
- Chrome: the default extension CSP disables WebAssembly. The minimum (most permissive) `extension_pages`
  policy is `script-src 'self' 'wasm-unsafe-eval'; object-src 'self';`
  (<https://developer.chrome.com/docs/extensions/reference/manifest/content-security-policy>, "Default Policy" and
  "Extension Pages Policy"). vexdom's `wxt.config.ts:24-56` declares no CSP today.
- Firefox MV2: "Manifest V2 extensions in Firefox can use WebAssembly without the use of 'wasm-unsafe-eval'.
  However, this behavior isn't guaranteed ... encouraged to declare 'wasm-unsafe-eval'"
  (<https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/content_security_policy>,
  Manifest V2 example).
- Extension SW: static `import` needs `"type": "module"`, and dynamic `import()` "is not supported"
  (<https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/basics>, "Import scripts"). BoltFFI
  output has no dynamic `import()` (scan above).
- Listeners must be registered synchronously at top level
  (<https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/events>). Top-level await is
  banned in SW scripts (W3C ServiceWorker resolution, implemented by Gecko as a SpiderMonkey flag:
  <https://bugzilla.mozilla.org/show_bug.cgi?id=1360870>, comment 5). So `await initialized` at module top level, as the
  README shows (`npm.rs:130-132`), **must not** be copied into the SW. Await it inside each handler.
- vexdom's background is `defineBackground(() => …)` without `type: 'module'`
  (`packages/extension/src/extension/entrypoints/background.ts:48`). WXT exposes `type: undefined | 'module'`
  (<https://wxt.dev/guide/essentials/entrypoints.html>, Background). It is **unverified** how Vite rewrites
  `new URL(…, import.meta.url)` in WXT's non-module background build. Avoid the question:
  `import init, {…} from '<m>.js'` and then
  `init(await fetch(browser.runtime.getURL('/<m>_bg.wasm')))`, with the `.wasm` placed in `public/`.

**Content scripts.** Out of scope by ADR-015. The BoltFFI glue itself has no BoltFFI-specific blocker: no
`window`/`document` and no `import.meta` in the core module. The page-CSP question is generic to wasm and not verified here.

## 3. Type support on the wasm target, against wasm-bindgen 0.2.130

| Feature | BoltFFI 0.32.1 (TS/wasm) | wasm-bindgen 0.2.130 |
|---|---|---|
| `Vec<u8>` | `Uint8Array` (`boltffi:docs/src/content/docs/types.mdx:63`). Returned bytes are **copied** out (`.slice()`, `module.ts:1021,1118,1330`). Tested in `examples/platforms/wasm/tests/bytes/mod.test.mjs:1-33` | Boxed number slices copy both ways into a `TypedArray` (<https://wasm-bindgen.github.io/wasm-bindgen/reference/types/boxed-number-slices.html>) |
| `String` | `string` (`types.mdx:39`) | `string`, copied |
| Records (nested) | `#[data]` struct becomes a TS `interface` with readonly fields plus a wire codec (`boltffi:boltffi_backend/templates/target/typescript/record.ts:3-26`). Nested round-trip tested (`tests/records/nested.test.mjs:1-19`) | `#[wasm_bindgen]` structs are opaque classes. Public fields get auto getters only if `Copy`, otherwise `getter_with_clone` or manual (<https://wasm-bindgen.github.io/wasm-bindgen/reference/types/exported-rust-types.html>). Plain-object structs need serde-wasm-bindgen or tsify (not built in) |
| Data enums | Tagged union `{ readonly tag: "V"; …fields }` (`templates/target/typescript/data_enum.ts:3-5`). Tested (`tests/enums/data_enum.test.mjs:1-30`) | Unit variants, or variants with **exactly one unnamed field** ("dynamic union"). Named fields are rejected: "enum variants cannot have named fields" (`wbg-macro-support:src/parser.rs:2776-2792`) |
| `Option<T>` | `T \| null` (`types.mdx:49`) | Supported per type (<https://wasm-bindgen.github.io/wasm-bindgen/reference/types.html>) |
| `Result<T, String>` | Throws a **plain `Error`**: `throw new Error(_module.takePackedWireString(...))` (`boltffi:boltffi_backend/src/target/typescript/mod.rs:1557`, a codegen test assertion). The docs claim `FfiException` (`errors.mdx:41,95-107`), which is **drift** | `Err` is thrown; needs `E: Into<JsValue>` (<https://wasm-bindgen.github.io/wasm-bindgen/reference/types/result.html>) |
| `Result<T, MyErr>` | Throws `MyErrException` with `.value: MyErr` (`record.ts:6-12`; `data_enum.ts:15-21`; `mod.rs:1567`) | Same mechanism; the error must be convertible to `JsValue` |
| `HashMap` | TS `Map` (`tests/collections/mod.test.mjs:4-13`) | Not native; serde |
| `i64`/`u64` | `bigint` (`types.mdx:25,29`) | `bigint` |
| Classes | `BoltFFIHandle` with explicit `dispose()` and a `FinalizationRegistry` fallback (`runtime/typescript/src/handle.ts:10-36`; `class.ts:1-6`). Default requires `Send + Sync`; `#[export(single_threaded)]` opts out (`boltffi_macros/src/expansion/wrapper/class.rs:364-388`). `&mut self` methods need `single_threaded` (`classes.mdx:699`) | Opaque class with `free()`, plus FinalizationRegistry when available (`wbg-cli:src/js/mod.rs:2415-2417`). No `Send` requirement |
| Panics | wasm32 is `panic=abort` by default (<https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html>, "Unwinding"). A panic is a `WebAssembly.RuntimeError` and **every pending future rejects** (`examples/platforms/wasm/tests/wasm_interop.test.mjs:68-75`). The instance must be discarded (`runtime/typescript/README.md:57-64`) | Same abort semantics |

Async, callbacks and streams are covered in §4-§6.

## 4. Async → Promise on wasm: who is the executor

**BoltFFI.** There is no Rust executor ("does not require a Rust async runtime ... the target language drives the
entire polling lifecycle"; `boltffi:docs/src/content/docs/async.mdx:63,360-364`). The flow:
1. JS calls the export, gets a future handle and polls it once synchronously (`module.ts:175-200`).
2. The Rust waker calls the imported `env.__boltffi_wake(handle)` (`boltffi_core/src/runtime/future.rs:22-26`).
3. JS records the handle and repolls on a `queueMicrotask` drain (`module.ts:89-111`).

So the **JS microtask queue is the executor**. Futures that need a reactor (tokio timers or IO) never wake. Wakeups must
come from JS: a callback, or a wasm-bindgen `JsFuture`.

The bound: `RustFuture<T: Send>` wraps `Pin<Box<dyn Future<Output = T> + Send + 'static>>`
(`future.rs:112,163-171`). The runtime README says the same: "BoltFFI async exports still require `Send`
futures. A wasm-bindgen `JsFuture` does not satisfy that bound" (`runtime/typescript/README.md:66-68`). The
upstream workaround is `wasm_bindgen_futures::spawn_local` plus a `futures_channel::oneshot` awaited by the
`Send` export (`boltffi:examples/demo/src/wasm_interop.rs:103-115`). That makes the crate depend on wasm-bindgen
and triggers the hybrid pack path (§1 step 3).

Cancellation: async exports take an options object with `signal` (an `AbortSignal`), and an abort raises
`BoltFFICancelledError` (`boltffi_backend/src/target/typescript/mod.rs:1370`; `module.ts:61-66`).

**wasm-bindgen.** `spawn_local<F: Future<Output = ()> + 'static>` and `future_to_promise<F: … + 'static>` have **no
`Send` bound** (`js-sys:src/futures/mod.rs:107-110,313-316`). Scheduling also uses `queueMicrotask`
(`js-sys:src/futures/queue.rs:63-69`). `wasm-bindgen-futures` 0.4.80 re-exports these from js-sys
(`wbg-futures:src/lib.rs:14`).

**Consequence for vexdom.** Anything `Rc`-based (capnp-rpc clients, `RefCell` state) cannot sit behind a BoltFFI
`async fn` without a bridge. wasm-bindgen takes it directly.

## 5. Callbacks and traits implemented in TS

- `#[export] trait` becomes a TS `interface`, with sync methods and `async` methods returning `Promise<T>`
  (`boltffi_backend/templates/target/typescript/callback.ts:1-4`). Tested with async traits that return records,
  options and results (`examples/platforms/wasm/tests/callbacks/async_traits.test.mjs:1-60`;
  Rust side `examples/demo/src/callbacks/async_traits.rs:7-18`; the demo declares the traits `Send + Sync`).
- Lifetime: a JS object lives in a `CallbackRegistry` `Map` with a refcount (`runtime/typescript/src/callback.ts:8-56`).
  Dropping the Rust side calls the `free` import, which releases it (`callback.ts` template `:17-19`;
  `boltffi_core/src/callback/wasm.rs:35-42`). Long-lived `Box<dyn Listener + Send>` parameters are supported
  (`boltffi_macros/src/expansion/contract/mod.rs:7206-7217`). JS-backed objects are force-marked `Send`
  (`boltffi_macros/src/expansion/wrapper/callback.rs:257,635`).
- Errors:
  - A callback whose Rust signature returns `Result` must **return `wireOk(v)` / `wireErr(e)`** from
    `@boltffi/runtime`, not a raw value or a throw.
  - A thrown JS error is wrapped by `writeUnexpectedCallbackError` (template `callback.ts:63,80,101`) and becomes
    the declared error through `From<UnexpectedFfiCallbackError>` (`boltffi_core/src/lib.rs:89-94`). In the demo that is
    `MathError::Overflow` (`tests/callbacks/errors.test.mjs:17-41`).
- Closures (`impl Fn(..)`) are supported as parameters (`docs/src/content/docs/callbacks.mdx:69,139`).
- **wasm-bindgen equivalent:** duck-typed imports with `extern "C" { type T; #[wasm_bindgen(method)] fn m(this: &T); }`
  (<https://wasm-bindgen.github.io/wasm-bindgen/reference/attributes/on-js-imports/method.html>), or `js_sys::Function`
  and `Closure` (<https://wasm-bindgen.github.io/wasm-bindgen/reference/passing-rust-closures-to-js.html>). There is no
  generated TS `interface`, and async callbacks are hand-rolled (`JsFuture` on the returned Promise).

## 6. Streams

- `#[ffi_stream(item = T)]` on a class method yields an `AsyncIterable`. `mode = "batch"` yields `popBatch()`, and
  callback subscriptions are also available. All three are tested on wasm (`examples/platforms/wasm/tests/classes/streams.test.mjs:1-50`).
- **Lossy by default:** each subscription has a 256-item ring buffer, and "When the buffer is full, new events are
  dropped" (`docs/src/content/docs/streaming.mdx:374-389`).
- The docs say TS streams need `experimental = ["typescript.async_streams"]`, or else stream methods are skipped
  (`experimental.mdx:67-71`). At HEAD the flag is registered (`boltffi_cli/src/config/experimental.rs:10-14`), but its only
  other use is a name formatter (`config/mod.rs:852`). The demo config does not enable it (`examples/demo/boltffi.toml`) yet
  streams are tested. Issue #40 "Complete TypeScript/WASM stream bridging" is still open
  (<https://github.com/boltffi/boltffi/issues/40>). Status: **unclear. Verify with `--deny-skipped`.**
- wasm-bindgen only converts *JS* async iterators into Rust `Stream`s (`js-sys:src/futures/stream.rs:1-5`). Rust→JS
  `AsyncIterable` is hand-written. For vexdom's watches, ADR-014 already plans capnp callback capabilities, so
  neither binding's streams are on the critical path.

## 7. Size

- **Runtime (measured):** `@boltffi/runtime@0.32.1` `dist/*.js` is 93,367 B raw. Bundled and minified with
  `bun build --minify` from `dist/index.js` it is **42 KB**, and **9,303 B** after `gzip -9`. This is an upper bound:
  `BoltFFIModule` (`module.ts`, 60 KB of the raw dist) is the bulk and will not tree-shake away.
- **Per-export glue:** **not measured** (no CLI). Shared per-class lifecycle code was moved into the runtime
  because it cost "roughly 300 bytes of identical glue each time" (`runtime/typescript/src/handle.ts:1-9`). Each
  record or enum emits a size/encode/decode codec (`record.ts:15-26`, `data_enum.ts:23-40`).
- **Wasm:** `-Os` wasm-opt plus a debug strip in release (§1). boltffi's default features pull `uuid` and `url`
  (`boltffi/Cargo.toml` `[features] default`; `boltffi_core/Cargo.toml`). Linking only keeps what is used, but set
  `default-features = false` unless needed (unmeasured).
- **wasm-bindgen:** no separate runtime package; glue is emitted per export by `wasm-bindgen-cli-support`. Not
  measured either. The S2 gate compares both on the same API.
- Upstream's speed claim ("up to 450x faster than wasm-bindgen") is about returning `Vec<Record>`, where
  wasm-bindgen materializes one JS class per element. On `Vec<u8>` (64 KiB) and scalars the two tie
  (`benchmarks/README.md:276-312`). This is not material for `ext-core`.

## 8. Stability and maintenance signals

- Repo `boltffi/boltffi` (Organization) was created 2025-12-23. It has 922 stars, 58 forks and MIT licence, was last pushed
  2026-11-25, and is not archived (`gh api repos/boltffi/boltffi`).
- **Release cadence:** 35 GitHub releases. The first was `0.1.0-rc1` on 2025-12-23 and `0.1.0` came on 2026-02-17. From 0.21.0
  (2026-03-31) to 0.32.1 (2026-10-30) there were 11 minor versions, each breaking under Cargo's 0.x semver.
  The 0.32.0 notes say "This is a breaking release". Required migrations included moving `#[export]` to structs,
  `scaffolding!()` in every crate, renamed symbols, and "Regenerate bindings for every target"
  (<https://github.com/boltffi/boltffi/releases/tag/v0.32.0>; `docs/src/content/docs/migrating.mdx:6-110`).
  `@boltffi/runtime` versions track the crate (npm `time` field).
- **Bus factor:** the contributors API counts `engali94` (Ali A. Hilal, the workspace author, `Cargo.toml:14`) at 1,401 of
  about 2,186 non-bot contributions (~64%). Next are `osa8bit` with 234 and `akesson` with 143
  (`gh api repos/boltffi/boltffi/contributors`).
- **Backlog:** 61 open issues and 24 open PRs (GitHub search, 2026-11-25). The WASM-labelled open issues are:
  - #40, streams
  - #202, `Vec<BlittableRecord>` with padding encoded with the wrong layout
  - #203, nested `Vec<Vec<isize>>` writer passes `Number` to `setBigInt64`
  - #529, fallible class constructors silently drop the error (reported on 0.26.1; not re-checked on 0.32.1)
- **CI:**
  - wasm demos run in Playwright on chromium, firefox and webkit, as **pages**
    (`examples/platforms/wasm/playwright.config.mjs:8`).
  - The wasm-bindgen interop matrix covers only `0.2.122` and `0.2.129` (`.github/workflows/ci.yml:290`). The
    current release is 0.2.130.
  - The toolchain is pinned to 1.97.1 (`rust-toolchain.toml:2`) on edition 2024 (`Cargo.toml:13`).
- **For contrast, wasm-bindgen:** 0.2.130 (2026-10-30) with roughly monthly releases (0.2.125→0.2.130, June→October 2026).
  It has 689M downloads and 9.2k stars, and is org-maintained (crates.io API; `gh api repos/wasm-bindgen/wasm-bindgen`). It must also be
  version-locked: the CLI refuses a module whose schema version differs (`wbg-cli:src/lib.rs:512-519`;
  `src/wit/mod.rs:3060-3069`).

## Semantic traps

1. **`Send` futures only.** An `async fn` export or async method cannot hold `Rc`, `RefCell` or capnp-rpc clients across
   `.await` (`future.rs:112,171`). Bridging through `spawn_local` requires wasm-bindgen, so you end up with two binding
   systems in one module (§4).
2. **The microtask queue is the executor.** Timers and IO futures from tokio never wake. Get wakeups from JS (§4).
3. **`web.js`/`bundler.js` fetch on import** through `import.meta.url` (`loader.js:9-12`). Import `<m>.js` and call
   `init(source)` explicitly. Never `await initialized` at SW top level: TLA is banned and listeners must register synchronously.
4. **Each JS realm owns one instance.** The SW, the offscreen document and every extension page each instantiate and hold separate
   state (`browser.ts:5-6`). A SW restart loses all wasm state: re-init on every wake and keep state in `storage`.
5. **A panic kills the instance.** The error is `WebAssembly.RuntimeError`, all pending futures reject and the instance must be discarded
   (`README.md:57-64`; `wasm_interop.test.mjs:68-75`). `BoltFFIPanicError` only appears where unwinding works.
6. **String errors are plain `Error`**, not the `FfiException` the docs show (`mod.rs:1557` vs `errors.mdx:41`).
   Typed errors are `XException` with `.value`.
7. **Result-returning TS callbacks must return `wireOk` / `wireErr`.** A throw becomes the error type's
   `From<UnexpectedFfiCallbackError>` value (`errors.test.mjs:17-41`). Pick that conversion deliberately.
8. **Streams drop events** when the 256-slot ring is full (`streaming.mdx:389`).
9. **Silent skipping.** Without `--deny-skipped`, unsupported declarations are left out of the TS (`cli.rs:153-158`).
10. **Runtime version `"*"`.** Pin `targets.wasm.typescript.runtime_version` to the exact crate version (`config/mod.rs:1096-1102`).
    The ABI check (version 3) does not catch runtime API drift.
11. **Hidden toolchain.** Release pack fails without `wasm-opt` (`config/mod.rs:1075`), and any wasm-bindgen dependency
    (getrandom `wasm_js`, uuid `js`, chrono `wasmbind`) triggers an exact-version `cargo install wasm-bindgen-cli`
    (`bindgen.rs:208-213`). getrandom 0.3 on `wasm32-unknown-unknown` needs the `wasm_js` backend, which depends on
    wasm-bindgen (`getrandom-0.3.4/Cargo.toml:50-53,74-77`; README `:87,97-98`).
12. **`&mut self` needs `single_threaded`.** A stateful class without it must use interior `Mutex` (`classes.mdx:699,1071`).
13. **Docs drift.** wasm-pack, the `import init from web.js` example, `FfiException` and the streams flag are all wrong or stale. Trust the
    templates and tests at the pinned commit.

## Spike checklist (S2)

Location: `spikes/s2-boltffi/` (throwaway). Use one test crate `s2core` and build it twice, once with `#[boltffi::export]` and once with
`#[wasm_bindgen]`, behind features.

0. **Setup:**
   - `rustup target add wasm32-unknown-unknown`
   - `cargo install boltffi_cli --version =0.32.1 --locked`
   - `wasm-opt` (binaryen)
   - `tsc` on `PATH`
   - `cargo install wasm-bindgen-cli --version =0.2.130 --locked`

   In `boltffi.toml`, set `runtime_version = "0.32.1"` and `npm.targets = ["web"]`.

   **Gate:** `boltffi pack wasm --release --deny-skipped` exits 0.
1. **API under test:**
   - `fn echo_bytes(Vec<u8>) -> Vec<u8>`, run with 2 MiB of screenshot-sized input
   - `fn parse(String) -> Result<Tree, ParseError>`, where `Tree` is a nested record with `Vec<Node>`, `Option<String>` and a data enum
     with named fields
   - `async fn delayed(ms: u32, timer: impl Timer) -> String`, with the `Timer` trait implemented in TS via `setTimeout`
   - `trait Sink { fn event(&self, e: Event); }`, registered once and called later
   - `#[export(single_threaded)]` class with `&mut self`

   **Gate:** every call round-trips with correct values and error class (`ParseErrorException` with `.value`).
2. **CSP scan.** Run `grep -E "eval\(|new Function|import\(|importScripts|document\.|window\."` on the packed `*.js` files.

   **Gate:** zero hits outside comments.
3. **Chrome MV3 SW:**
   - minimal WXT extension, background `type: 'module'`
   - CSP `extension_pages: "script-src 'self' 'wasm-unsafe-eval'; object-src 'self'"`
   - `init(await fetch(chrome.runtime.getURL('/s2core_bg.wasm')))` inside the first handler
   - listeners registered synchronously

   **Gates:**
   - no CSP violation in `chrome://extensions` errors
   - all step-1 calls pass from a `runtime.onMessage` handler
   - after `chrome://serviceworker-internals` → Stop, the next message re-inits and passes
   - record cold-start `init` time (ms)
4. **Same in a non-module WXT background build** (vexdom's current setting, `background.ts:48`).

   **Gate:** loads. If not, record the error. Its outcome decides whether vexdom must switch to `type: 'module'`.
5. **Firefox MV2 background** (`wxt build -b firefox`) with `content_security_policy: "script-src 'self' 'wasm-unsafe-eval'"`.

   **Gate:** the same calls pass, and `web-ext lint` is clean.
6. **Async with `!Send`.** Try an `async fn` export that awaits a `Rc<RefCell<_>>`-held value.

   **Expected:** compile error. Then implement the `spawn_local` + oneshot bridge (`wasm_interop.rs:103-115`).

   **Gate:** it works, and pack auto-handles wasm-bindgen glue at 0.2.130, a version not in upstream CI.
7. **Panic.** Trigger a panic in a sync and an async export.

   **Gate:** a `RuntimeError` surfaces, and the extension recovers by re-instantiating (not by reloading the extension).
8. **Size.** For each of BoltFFI and wasm-bindgen, after the WXT production build, record:
   - `.wasm` raw and gzip
   - glue JS min and gzip
   - BoltFFI runtime share

   **Gate:** the BoltFFI total is ≤ wasm-bindgen total + 30 KB gzip. Otherwise record the delta for ADR-016.
9. **Ergonomics.** Count the hand-written lines needed to expose the same API through wasm-bindgen: serde or tsify for
   `Tree` and enums, the duck-typed `Timer`/`Sink` imports, and Promise plumbing.

   **Gate:** none. Record the number for the ADR.

**Overall:** any failure in steps 2, 3, 5 or 7 means wasm-bindgen, per ADR-016. If step 6 fails, so that no workable path to
`Rc`/capnp state exists, that also means wasm-bindgen when S1 puts capnp-rpc in wasm.

## Recommendation

Run S2 as planned, but expect wasm-bindgen to win for vexdom:
- BoltFFI loads cleanly in principle: own ABI, no eval, `instantiate` from bytes.
- Its strengths are typed records, data enums, maps and TS-implemented traits with generated interfaces. These do cut
  hand-written glue compared with wasm-bindgen.
- Three things weigh against it:
  1. The `Send`-future rule clashes with an `Rc`-based capnp-rpc client living in the same wasm (ADR-014 + ADR-015). The
     bridge needs wasm-bindgen anyway.
  2. rust-anything's wasm bridge, which ADR-015 uses for record/diff/gate, already emits wasm-bindgen.
  3. The project is young, single-maintainer-heavy and breaks every minor release, with docs that lag the code.

Keep ADR-016's binding-agnostic `ext-core` API. If S2 passes every gate *and* `ext-core` exposes no `!Send` async
surface (capnp stays in TS or behind sync calls), BoltFFI is acceptable, pinned with `=0.32.1` for both crate and runtime.
Otherwise choose wasm-bindgen and add `serde-wasm-bindgen`/`tsify` for records and enums.
