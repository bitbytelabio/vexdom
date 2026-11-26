/**
 * ext-core (`crates/ext-core`): the Bridge link's capnp-rpc, compiled to wasm with BoltFFI
 * (ADR-017). `bun run ext-core:build` writes the glue to `./ext-core/` and the module to
 * `public/ext-core.wasm`; `./pure.ts` loads it and holds the pure half (P9).
 */
export {
  linkClose,
  linkEvents,
  linkFeed,
  linkHello,
  linkOpen,
  linkProve,
  linkPump,
  linkReply,
  linkReplyArtifact,
  linkReplyScreenshot,
  linkRequest,
  linkTrust,
  type ExtensionRequest,
  type LinkEvent,
} from './ext-core/ext_core.js';
export { loadCore } from './pure';
