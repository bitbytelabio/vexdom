# Handshake crypto: byte-exact spec for the Rust port

Scope: `packages/extension/src/lib/actions/handshake.ts` (all 140 lines), its daemon callers in `src/daemon/daemon.ts`
(`greet`/`settle`, `hasValidToken`), `src/daemon/auth-store.ts` and `src/daemon/pairing-desk.ts`
(codes, grants, session keys), and the extension side `packages/extension/src/lib/bridge/socket.ts`. Bar: ADR-008
(`DECISIONS.md:68-73`). Randomness is normalized under ADR-008, so only its *shape* has to match;
every derived value (secret, proofs, sealed key) must match byte for byte.

Evidence: TS run under Node v26.13.1 (WebCrypto is `globalThis.crypto`,
[node v26.13.1 doc/api/globals.md:405](https://github.com/nodejs/node/blob/v26.13.1/doc/api/globals.md#crypto)),
Rust run against pbkdf2 0.13.0, hmac 0.13.0, sha2 0.11.1, base64 0.23.1 (rustc/cargo 1.100.0) in a
scratch crate outside the repo. Section 8 lists the observed vectors; Rust reproduced every one.

## 1. Frames and order (daemon side)

| Step | Who | Frame / action | Source |
|---|---|---|---|
| 0 | ext | WS upgrade with `Origin: chrome-extension://…` (or `moz-`/`safari-web-`) | `daemon.ts:108,287-303` |
| 1 | ext | `{"t":"hello","protocolVersion":26,"extensionVersion","manifestHash","auth":{"kind"},"nonce", …identity}` | `socket.ts:578-588` |
| 2 | daemon | reject: close 1002 `expected hello` / protocol refusal text / `expected a nonce` / `expected an install id` | `daemon.ts:329-349` |
| 3 | daemon | `{"t":"challenge","nonce":<serverNonce>}` | `daemon.ts:359-366` |
| 4 | ext | `{"t":"prove","proof":<clientProof>}` | `socket.ts:676-684` |
| 5 | daemon | no `prove`: close 1002 `expected a proof`; wrong proof: `{"t":"unauthorized","reason","retryable","code"}` then close 4401 `unauthorized` | `daemon.ts:320-327,368-403` |
| 6 | daemon | `{"t":"welcome","daemonVersion","manifestHash","manifestInSync",["sealedSessionKey"],"protocolVersion","proof"}` | `daemon.ts:445-452,569` |
| any | daemon | an exception anywhere in `greet` (e.g. a zero-length HMAC key, section 4.2): close 1011 `handshake failed` | `daemon.ts:312-317` |

Each wait is 10 s (`HANDSHAKE_TIMEOUT_MS`, `daemon.ts:107`, `nextFrame` at `daemon.ts:1138-1149`).
Welcome key order is fixed by the object spread: `t`, the four `welcome` fields in declaration
order, then `protocolVersion`, `proof`. `sealedSessionKey: undefined` is dropped by JSON.stringify
([ES2027 §25.5.4.5 SerializeJSONObject](https://tc39.es/ecma262/multipage/structured-data.html#sec-serializejsonobject)
skips a property whose SerializeJSONProperty is undefined;
[§25.5.4.2](https://tc39.es/ecma262/multipage/structured-data.html#sec-serializejsonproperty) returns
undefined for undefined). `welcome.protocolVersion` is not covered by `serverProof`
(`handshake.ts:57-65` hashes only the four claims). `research/capnp-wasm.md:345-354` makes the same point
about a future `transport` field.

## 2. Encodings

### 2.1 String to bytes
`const encoder = new TextEncoder()` (`handshake.ts:11`), and every key, salt and message goes through
`encoder.encode(...)` (`handshake.ts:31-32,96,110`). `encode(input)` takes a USVString and returns
UTF-8 ([WHATWG Encoding §7.4](https://encoding.spec.whatwg.org/#interface-textencoder)). The USVString
conversion replaces lone surrogates with U+FFFD
([WebIDL §3.2.12](https://webidl.spec.whatwg.org/#js-USVString);
[Infra "convert to a scalar value string"](https://infra.spec.whatwg.org/#javascript-string-convert)).
Rust: `str::as_bytes()` is exactly this UTF-8 for any valid `String`. Observed: a transcript whose
`extensionVersion` is `"\ud800"` gives the same proof as `"�"` (section 8).

### 2.2 base64url, no padding (encode)
`encode()` (`handshake.ts:126-130`): bytes become a Latin-1 string, then `btoa` (forgiving-base64
encode, i.e. RFC 4648 §4; [HTML §8.3 btoa](https://html.spec.whatwg.org/multipage/webappapis.html#dom-btoa),
[Infra forgiving-base64 encode](https://infra.spec.whatwg.org/#forgiving-base64-encode)), then
`+`→`-`, `/`→`_`, and trailing `=` stripped. That is RFC 4648 §5 without padding, the same as
`base64::engine::general_purpose::URL_SAFE_NO_PAD` (`base64-0.23.1/src/engine/general_purpose/mod.rs:472`,
[docs.rs](https://docs.rs/base64/0.23.1/base64/engine/general_purpose/constant.URL_SAFE_NO_PAD.html)).
Node's `Buffer#toString('base64url')` (session keys, lock token) also "will omit padding"
([Node buffer docs, "Buffers and character encodings"](https://nodejs.org/api/buffer.html#buffers-and-character-encodings)).
Lengths: 16 B → 22 chars, 24 B → 32, 32 B → 43, 43 B → 58, 70 B → 94.

### 2.3 base64url decode (`openSessionKey`, extension only)
`decode()` (`handshake.ts:132-140`): `-`→`+`, `_`→`/`, then `atob`, which runs forgiving-base64 decode
and throws on failure ([HTML §8.3 atob](https://html.spec.whatwg.org/multipage/webappapis.html#dom-atob)).
[Infra forgiving-base64 decode](https://infra.spec.whatwg.org/#forgiving-base64-decode): (1) remove ASCII
whitespace (TAB, LF, FF, CR, SPACE; [Infra §4.6](https://infra.spec.whatwg.org/#ascii-whitespace)); (2) if
length % 4 == 0, drop one or two trailing `=`; (3) length % 4 == 1 fails; (4) anything outside
`[A-Za-z0-9+/]` fails; (5) leftover 12 or 18 bits: the extra 4 or 2 bits are **discarded, even if
non-zero**. So the TS decoder takes mixed standard and url-safe alphabets, embedded whitespace, and
padding only when it completes a quad. Observed: `sealed+"="` (length 59) → `null`, `" "+sealed` →
opened, standard alphabet → opened, `"A"` and `"!!!!"` → `null`.
`base64` has no preset that matches this. `URL_SAFE_NO_PAD` rejects non-zero trailing bits
(`InvalidLastSymbol`), whitespace and any padding (observed). `*_INDIFFERENT` takes `"AA="`, which atob
rejects (observed). Exact Rust:

```rust
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
const FORGIVING: GeneralPurpose = GeneralPurpose::new(&base64::alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_decode_allow_trailing_bits(true));          // mod.rs:372 cites forgiving-base64

/// handshake.ts:132-140 = atob(value with - → +, _ → /)
fn decode(value: &str) -> Option<Vec<u8>> {
    let mut d: String = value.chars().filter(|c| !matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' '))
        .map(|c| match c { '-' => '+', '_' => '/', c => c }).collect();
    if d.chars().count() % 4 == 0 {
        if d.ends_with("==") { d.truncate(d.len() - 2) } else if d.ends_with('=') { d.truncate(d.len() - 1) }
    }
    if d.chars().count() % 4 == 1 { return None; }
    if !d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/') { return None; }
    FORGIVING.decode(d).ok()
}
```

The Rust daemon never opens a sealed key; only the extension does (`socket.ts:702`). This decoder is
needed only if `openSessionKey` moves into `ext-core` (ADR-015).

### 2.4 Bytes to string (`openSessionKey`, extension only)
`new TextDecoder()` (`handshake.ts:12`) defaults to utf-8 with `fatal = false` and `ignoreBOM = false`
([Encoding §7.2](https://encoding.spec.whatwg.org/#interface-textdecoder)). So invalid sequences become
U+FFFD ("replacement" error mode, [§4.1](https://encoding.spec.whatwg.org/#concept-encoding-process))
and **a leading EF BB BF is stripped** (serialize step, [§7.1](https://encoding.spec.whatwg.org/#interface-mixin-textdecodercommon)).
The UTF-8 decoder follows Unicode's "Best Practices for Using U+FFFD"
([§8.1.1](https://encoding.spec.whatwg.org/#utf-8-decoder)). `String::from_utf8_lossy` replaces invalid
sequences ([std docs](https://doc.rust-lang.org/std/string/struct.String.html#method.from_utf8_lossy))
but keeps the BOM. Observed:
`from_utf8_lossy(b"\xEF\xBB\xBFa\xF0\x90\x80b\xC0\xAFc\xED\xA0\x80d")` = `"\u{feff}a�b��c���d"`.
A port must strip one leading U+FEFF after the lossy decode. Not recorded against TextDecoder.

### 2.5 Values to strings in transcripts
- `join(...parts)` is `Array.prototype.join('|')` (`handshake.ts:122-124`). Per element, undefined and
  null become `""`, anything else goes through `ToString`
  ([ES2027 §23.1.3.18.1 JoinIndexedProperties](https://tc39.es/ecma262/multipage/indexed-collections.html#sec-array.prototype.join)).
- The daemon **does not type-check** `hello.extensionVersion` or `hello.manifestHash` (`daemon.ts:350-365`;
  `parseFrame` only checks that `t` is a string, `packages/extension/src/lib/actions/protocol.ts:362-369`). Whatever JSON
  value arrives is coerced: missing/null → `""`, `true` → `"true"`, a number → Number::toString, an
  array → its elements joined with `,` (recursively, null → `""`), an object → `"[object Object]"`
  ([§20.1.3.6](https://tc39.es/ecma262/multipage/fundamental-objects.html#sec-object.prototype.tostring)).
  Observed: `{extensionVersion: undefined, manifestHash: null}` proves the same as `""`/`""`, and
  `["a","b"]` the same as `"a,b"` (section 8). A client that does the same coercion is let in, and its
  raw value lands in `Install.extensionVersion` and then `auth.json` (`daemon.ts:351-356`,
  `auth-store.ts:110-128`).
- `String(transcript.protocolVersion)` (`handshake.ts:103`): `protocolRefusal` has already required
  `typeof === 'number'` and `>= 22` (`daemon.ts:127-137`), so this is
  [Number::toString(x, 10)](https://tc39.es/ecma262/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-tostring)
  (§6.1.6.1.20): `26.0` → `"26"`, `2.6e1` → `"26"`, `1e21` → `"1e+21"`, `-0` → `"0"`. JSON `1e400` parses
  to Infinity, passes the check, and gives `"Infinity"`.
- `String(welcome.manifestInSync)` gives `"true"`/`"false"` (`handshake.ts:61`). `String(block)` is
  the decimal integer `"0"`, `"1"`, … (`handshake.ts:116`).
- `welcome.sealedSessionKey ?? ''` (`handshake.ts:62`): absent → `""`.

## 3. Transcript strings (separator `|`, no escaping)

`T = protocolVersion | extensionVersion | manifestHash | clientNonce | serverNonce` (`handshake.ts:101-109`).
The daemon uses **the extension's** protocol number, `hello.protocolVersion` (`daemon.ts:358-365`), and the
extension uses its own constant (`socket.ts:659-666`). Both should be 26.

| Use | Exact message (UTF-8) | Source |
|---|---|---|
| PBKDF2 salt | `vexdom/pair|<clientNonce>|<serverNonce>` | `handshake.ts:32` |
| client proof | `vexdom/client|<T>|` (7 fields, empty last one, so a trailing `|`) | `handshake.ts:41-43,93-111` |
| server proof | `vexdom/server|<T>|<daemonVersion>|<manifestHash>|<true|false>|<sealedSessionKey or "">` | `handshake.ts:57-65` |
| seal block n | `vexdom/seal|<T>|<n>`, n = 0,1,2,… in decimal | `handshake.ts:113-120` |

Because nothing is escaped, a field containing `|` is ambiguous. That is the design, so reproduce it
as is. Worked example for T0 (section 8): the client message is
`vexdom/client|26|0.9.2|1a2b3c4d|AAAAAAAAAAAAAAAAAAAAAA|_____________________w|`.

## 4. Primitives and parameters

### 4.1 PBKDF2 (`pairingSecret`, `handshake.ts:30-39`)
- Password: the UTF-8 bytes of the code, imported `raw` for `PBKDF2` with `extractable=false` and
  usages `['deriveBits']`. That matches [WebCrypto §34.4.2](https://w3c.github.io/webcrypto/#pbkdf2-operations-import-key),
  which does no length check. Observed: an empty password derives fine.
- PRF HMAC-SHA-256, salt as in section 3, `iterations = 250_000` (`handshake.ts:9`), `length = 256` bits,
  so dkLen = 32 ([§34.4.1](https://w3c.github.io/webcrypto/#pbkdf2-operations-derive-bits): RFC 8018 §5.2
  with dkLen = length/8; OperationError if the length is not a multiple of 8 or the iteration count is 0).
- **Output: `secret = base64url_nopad(dk)`, a 43-char ASCII string.** That string, not the 32 raw
  bytes, is the HMAC key in every later step.
- Rust: `pbkdf2::pbkdf2_hmac::<sha2::Sha256>(code.as_bytes(), salt.as_bytes(), 250_000, &mut [0u8; 32])`
  (`pbkdf2-0.13.0/src/lib.rs:224-227`, [docs.rs](https://docs.rs/pbkdf2/0.13.0/pbkdf2/fn.pbkdf2_hmac.html)).
  Its core is RFC 8018: a block index `(i + 1).to_be_bytes()` and an XOR over `rounds` iterations
  (`lib.rs:119-142`). The `hmac` feature is on by default (`pbkdf2-0.13.0/Cargo.toml:51`).
- Cost: the daemon tries the held grant first, then every live pairing code (up to 3), one PBKDF2
  each, in order (`daemon.ts:416-431`, `auth-store.ts:17,80`). In TS each one is an awaited Promise. In
  Rust, run it in `spawn_blocking` so 250k iterations do not stall the async runtime.

### 4.2 HMAC (`tag`, `handshake.ts:93-111`)
- Key: `encoder.encode(secret)`, imported `raw` as HMAC/SHA-256. The secret is either the 43-char
  pairing secret or the 43-char session key (`auth-store.ts:115`), so the key is 43 ASCII bytes. That is
  shorter than the 64-byte SHA-256 block, so it is not pre-hashed.
- **Zero-length key: WebCrypto throws DataError** ([§31.6.4](https://w3c.github.io/webcrypto/#hmac-operations-import-key):
  "If length is zero then throw a DataError"). Observed in Node: `DataError: Zero-length key is not
  supported`. RustCrypto accepts an empty key (observed `new_from_slice(b"")` → `Ok`). A session whose
  `key` is `""` in `auth.json` therefore makes TS close **1011 `handshake failed`**, while a naive port
  computes a proof and sends **4401**. The port must refuse an empty key the same way.
- Sign: HMAC per NIST SP 800-224 §2 ([§31.6.1](https://w3c.github.io/webcrypto/#hmac-operations-sign)),
  32-byte output used whole, with no truncation.
- Rust: `Hmac::<Sha256>::new_from_slice(secret.as_bytes())` then `update(msg)`, then
  `finalize().into_bytes()` (`hmac-0.13.0` README example; `pub use digest::{KeyInit, Mac}` at `src/lib.rs:20`).

### 4.3 Proofs
- `clientProof = b64url(tag(secret, 'vexdom/client', T, ''))` (`handshake.ts:41-43`).
- `serverProof = b64url(tag(secret, 'vexdom/server', T, claims))` (`handshake.ts:57-65`). The daemon
  computes it over its own `welcome` object (`daemon.ts:445-452`), the extension over the frame it
  received (`socket.ts:696`).

### 4.4 Sealing the session key (`sealSessionKey` / `openSessionKey`, `handshake.ts:67-82,113-120`)
- `clear = UTF-8(sessionKey)`. The session key is 43 ASCII chars (32 random bytes, base64url;
  `auth-store.ts:115`), so it is 43 bytes.
- Keystream: 32-byte blocks. Block n = `tag(secret, 'vexdom/seal', T, String(n))`. The counter is
  **decimal ASCII text appended as the last field**, not a binary big-endian integer, and starts at 0.
  The last block is cut to `length - offset` (`subarray(0, Math.min(32, length - offset))`). 43 bytes
  take blocks 0 and 1 (32 + 11).
- `sealed = b64url(clear XOR stream)`, 58 chars for a 43-byte key. No MAC over the ciphertext:
  integrity comes from `serverProof`, which covers `sealedSessionKey`.
- The daemon seals only when pairing by code or grant (`daemon.ts:377-392`). With session auth, the
  welcome carries no `sealedSessionKey`.

### 4.5 Constant-time comparisons
- `sameProof(offered, expected)` (`handshake.ts:84-91`): returns false unless `offered` is a string;
  returns early when the **UTF-16 lengths** differ; otherwise ORs the XOR of each pair of code units.
  `expected` is base64url ASCII, so the boolean equals UTF-8 byte equality. Rust:
  `offered.as_str().is_some_and(|o| bool::from(o.as_bytes().ct_eq(expected.as_bytes())))`. `subtle`'s
  slice `ct_eq` "short-circuits if the lengths of the input slices are different"
  (`subtle-2.6.1/src/lib.rs:289-296`, [docs.rs](https://docs.rs/subtle/2.6.1/subtle/trait.ConstantTimeEq.html)),
  which is the same timing shape. `Mac::verify_slice` (`digest-0.11.3/src/mac.rs:139-147`) compares raw
  tag bytes, but the TS compares base64 strings. Either gives the same boolean, so do whichever is
  simpler.
- Control-socket token (`daemon.ts:305-310`): `header.replace(/^Bearer\s+/i, '')`, then
  `Buffer.from(...)` (UTF-8), a length check, then `timingSafeEqual`. `timingSafeEqual` "throws if `a`
  and `b` have different byte lengths", hence the length check before it
  ([node v26.13.1 doc/api/crypto.md:6701-6726](https://github.com/nodejs/node/blob/v26.13.1/doc/api/crypto.md#cryptotimingsafeequala-b)).
  Node hands header bytes over as Latin-1 code units. Observed: `Authorization: bEaReR<0xA0>tok`
  arrives as code units `…52 a0 74…`, and `\s` strips U+00A0, leaving `"tok"`. Rust works on header
  *bytes*. Strip ASCII-case-insensitive `bearer` followed by one or more of
  `{09,0A,0B,0C,0D,20,A0}` (the JS `\s` members that are Latin-1, js-regex-and-utf16.md section 1;
  0x85 is **not** one), then `ct_eq` the rest against the token bytes. Non-ASCII bytes left over cannot
  match the ASCII token in either implementation, so the UTF-8 re-encoding does not change the result.

## 5. Randomness (shapes the ADR-008 normalizer must recognize)

| Value | TS | Bytes → text | Source |
|---|---|---|---|
| client nonce, server nonce | `crypto.getRandomValues(new Uint8Array(16))` → `encode` | 16 → 22 chars base64url | `handshake.ts:1,22-24` |
| nonce check | `typeof string && length >= 16` (**UTF-16 units**; observed `"😀"×8` passes) | – | `handshake.ts:26-28` |
| session key | `randomBytes(32).toString('base64url')` | 43 chars | `auth-store.ts:115` |
| control token | `randomBytes(24).toString('base64url')` per daemon | 32 chars | `daemon.ts:228` |
| pairing code | 8 × `CODE_ALPHABET[randomInt(29)]`, alphabet `ABCDEFGHJKMNPQRSTWXYZ23456789` (no I L O U V 0 1) | 8 chars, log2(29^8) ≈ 38.9 bits | `auth-store.ts:9-10,70,77` |
| grant code | same alphabet, 26 chars, ≈ 126.3 bits, used as a PBKDF2 password like a code | 26 chars | `auth-store.ts:12,73`, `pairing-desk.ts:88` |
| pair-request id / nonce | `randomBytes(6)` / `randomBytes(16)` base64url | 8 / 22 chars | `pairing-desk.ts:105,112` |
| fence tag, secrets tag | `randomBytes(6)` / `randomBytes(8)` hex | 12 / 16 chars | `guardrails/fence.ts:27-29`, `guardrails/secrets.ts:20` |

`randomInt` has **no modulo bias**. It draws 48-bit big-endian values from a 6 KiB cache filled by
`randomFillSync`, rejects `x >= RAND_MAX - RAND_MAX % range` with `RAND_MAX = 0xFFFF_FFFF_FFFF`, and
returns `x % range` ([node v26.13.1 lib/internal/crypto/random.js:239-306](https://github.com/nodejs/node/blob/v26.13.1/lib/internal/crypto/random.js#L239-L306);
docs: "This implementation avoids modulo bias", `doc/api/crypto.md:6221-6242`). Rust choices:
- `rand 0.10`'s `IndexedRandom::choose` calls `random_range(..len)` (`rand-0.10.3/src/seq/slice.rs:52-61`).
  Without the `unbiased` feature that is "Canon's method, biased", with at worst 1 in 2^64 samples
  affected for 64-bit types (`rand-0.10.3/src/distr/uniform_int.rs:59-64,185-189`). Enable
  `features = ["unbiased"]` (`Cargo.toml:84`), or use `Uniform` ("Lemire's method, unbiased",
  `uniform_int.rs:149`).
- Simplest exact match: port Node's sampler on `getrandom::fill` (`getrandom-0.4.3/src/lib.rs:87`):

```rust
const RAND_MAX: u64 = 0xFFFF_FFFF_FFFF;                      // random.js:239
fn random_int(range: u64) -> u64 {                           // random.js:280-305
    let limit = RAND_MAX - (RAND_MAX % range);
    loop {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b[2..]).expect("OS RNG");      // 6 bytes, read big-endian
        let x = u64::from_be_bytes(b);
        if x < limit { return x % range; }
    }
}
const CODE_ALPHABET: &[u8; 29] = b"ABCDEFGHJKMNPQRSTWXYZ23456789";
fn code_of(len: usize) -> String { (0..len).map(|_| CODE_ALPHABET[random_int(29) as usize] as char).collect() }
```

## 6. Rust mapping

| Primitive | Crate (latest stable, crates.io 2026-11-25) | API |
|---|---|---|
| PBKDF2-HMAC-SHA256 | `pbkdf2 = "0.13.0"` | `pbkdf2::pbkdf2_hmac::<Sha256>(pw, salt, 250_000, &mut [u8; 32])` |
| HMAC-SHA256 | `hmac = "0.13.0"`, `sha2 = "0.11.1"` | `Hmac::<Sha256>::new_from_slice`, `Mac::update`, `finalize().into_bytes()` |
| constant-time eq | `subtle = "2.6.1"` | `<[u8]>::ct_eq` |
| base64url no pad | `base64 = "0.23.1"` | `URL_SAFE_NO_PAD.encode`; forgiving decode as in section 2.3 |
| OS randomness | `getrandom = "0.4.3"` (or `rand = "0.10.4"` with `unbiased`) | `getrandom::fill` |
| JS Number → string | `ryu-js = "1.0.4"` | `ryu_js::Buffer::new().format(f64)` (observed: `26.0`→`26`, `-0`→`0`, `1e21`→`1e+21`, `1e20`→`100000000000000000000`, `NaN`, `Infinity`) |

Sketch, verified to reproduce every vector in section 8:

```rust
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

pub const PAIRING_ITERATIONS: u32 = 250_000;

/// Each field is already the JS string the TS would join (section 2.5).
pub struct Transcript { pub protocol_version: String, pub extension_version: String,
                        pub manifest_hash: String, pub client_nonce: String, pub server_nonce: String }

#[derive(Debug)] pub struct DataError; // WebCrypto: zero-length HMAC key → greet throws → close 1011

fn b64(b: &[u8]) -> String { URL_SAFE_NO_PAD.encode(b) }

pub fn pairing_secret(code: &str, t: &Transcript) -> String {
    let salt = ["vexdom/pair", &t.client_nonce, &t.server_nonce].join("|");
    let mut dk = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(code.as_bytes(), salt.as_bytes(), PAIRING_ITERATIONS, &mut dk);
    b64(&dk)
}

fn tag(secret: &str, label: &str, t: &Transcript, extra: &str) -> Result<[u8; 32], DataError> {
    if secret.is_empty() { return Err(DataError); }
    let msg = [label, &t.protocol_version, &t.extension_version, &t.manifest_hash,
               &t.client_nonce, &t.server_nonce, extra].join("|");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(msg.as_bytes());
    Ok(mac.finalize().into_bytes().into())
}

pub fn client_proof(secret: &str, t: &Transcript) -> Result<String, DataError> {
    Ok(b64(&tag(secret, "vexdom/client", t, "")?))
}

pub fn server_proof(secret: &str, t: &Transcript, daemon_version: &str, manifest_hash: &str,
                    in_sync: bool, sealed: Option<&str>) -> Result<String, DataError> {
    let claims = [daemon_version, manifest_hash, if in_sync { "true" } else { "false" }, sealed.unwrap_or("")].join("|");
    Ok(b64(&tag(secret, "vexdom/server", t, &claims)?))
}

pub fn seal_session_key(secret: &str, t: &Transcript, session_key: &str) -> Result<String, DataError> {
    let clear = session_key.as_bytes();
    let mut out = Vec::with_capacity(clear.len());
    for (n, chunk) in clear.chunks(32).enumerate() {
        let block = tag(secret, "vexdom/seal", t, &n.to_string())?;
        out.extend(chunk.iter().zip(block).map(|(c, k)| c ^ k));
    }
    Ok(b64(&out))
}
```

`Transcript` construction from `hello` is where JS semantics leak in (section 2.5). Keep `hello` as a
`serde_json::Value` and build each field with a `js_to_string(&Value)` that copies `join`/`ToString`
(null/missing → `""`, number → `ryu_js` on `as_f64()`, array → recursive `,`-join, object →
`"[object Object]"`). Do not deserialize `hello` into a typed struct: that turns accepted TS inputs into
parse failures.

## 7. Where Rust output can differ

1. **The HMAC key is the base64url text of the PBKDF2 output**, not the 32 bytes (`handshake.ts:38,96`).
   Using `dk` directly as the key changes every proof.
2. **Seal counter**: the decimal string `"0"`, `"1"` as the last `|` field, not `u32::to_be_bytes`. A
   CTR-mode helper crate would differ.
3. **Zero-length secret**: DataError, so close 1011 (section 4.2).
4. **Number formatting** in the transcript and in refusal and log text: `serde_json`'s writer prints
   `26.0` for an f64 26 and `-0.0` for -0 (observed `[26.0,1e+21,…,-0.0]`). JS prints `26` and `0`. Use
   `ryu-js` on `as_f64()`.
5. **JSON that JS accepts and serde_json rejects**: a lone surrogate escape (`"\ud800"` →
   `unexpected end of hex escape`, observed; `serde_json-1.0.154/src/read.rs:951-1023` errors whenever
   `validate`) and `1e400` (`number out of range`, observed; JS gives Infinity). For those inputs TS goes
   on with the handshake while a serde_json port closes 1002 `expected hello`. For crypto parity alone,
   decoding lone surrogates as U+FFFD gives the same bytes TextEncoder would hash (section 2.1).
   Storage parity in `auth.json` would also need the original `\udXXX` written back out.
6. **Frame decoding**: `parseFrame(String(raw))` (`daemon.ts:1145`). `ws` 8 emits `message` with `data`
   and an `isBinary` flag, and the daemon ignores the flag
   ([ws doc/ws.md "Event: 'message'"](https://github.com/websockets/ws/blob/master/doc/ws.md#event-message)).
   A **binary** frame holding JSON is therefore accepted as `hello`, after a lossy UTF-8 decode. A
   tungstenite-based port must accept `Message::Binary` during the handshake too. ADR-018 later reserves
   binary frames for capnp, *after* `welcome`.
7. **`isNonce` length** is UTF-16 code units: `s.encode_utf16().count() >= 16`, not `s.len()`.
8. **TextDecoder BOM strip** (section 2.4) and **forgiving base64** (section 2.3), if `openSessionKey` is ported.
9. **Typed struct vs coercion** for `extensionVersion`/`manifestHash` (section 2.5).

## 8. Test vectors

Observed under Node v26.13.1 on an unmodified copy of `handshake.ts`, and reproduced exactly by the
Rust sketch. PBKDF2 and HMAC were also cross-checked with `node:crypto` `pbkdf2Sync`/`createHmac`.

`T0 = {"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"}`,
`S43 = "S"×43`.

| Call | Result |
|---|---|
| `pairingSecret("ABCD2345", T0)` | `bHj22qBUKo2-HZgUPOTc6SiIHf8Y4m1UmHtixx8fEto` |
| `clientProof(<that>, T0)` | `1IrbKWjEXM59Shtx7gMpzLbuqQiaKAG8RJadvd1QtsU` |
| `sealSessionKey(<secret>, T0, S43)` (2 blocks) | `zR6r4OCmRJs9Cj_HswPDQBM1uXNLBjKZRw0WBCbkVJIHg6IkujXxZwy3FA` |
| `serverProof(<secret>, T0, {daemonVersion:"0.9.2", manifestHash:"1a2b3c4d", manifestInSync:true, sealedSessionKey:<sealed>})` | `YmGvK2x55BPsz5gKdJSmimAg7rXp6T9JjDbjUGMzfDc` |
| `serverProof(S43, T0, {…, manifestInSync:false})` (no seal) | `tSsbrc2OIu5c_C0RY1tEYh2cnmy18O4E0NiDZU0tLTg` |
| `clientProof(S43, T0)` | `W75TpCxycM9M3U93T7C46pYi6gDGdFvlMEABBy6bsag` |
| `sealSessionKey(<secret>, T0, "k"×70)` (3 blocks, last cut to 6) | `9SaT2NiefKMFMgf_izv7eCsNgUtzPgqhfzUuPB7cbKo_u5ocgg3JXzSPLBjbURPgtLPSFKpPjomURp6Dx1pAUQexEf5Y4A` |
| `pairingSecret("", T0)` | `qzPSJbBeyqYkcDCGP53PzrhzbJpAq0GVf1EovUpNCKo` |
| `clientProof(S43, {…T0, extensionVersion: undefined, manifestHash: null})` = same with `""`,`""` | `bQIDv-bU6OQ7NjvgzH_sGaCkHq1tlsQCsflzsRQbDNQ` |
| `clientProof(S43, {…T0, extensionVersion:["a","b"]})` = with `"a,b"` | `pOdvGBlCwZLQoAQARRKYnPl5P_1YFSZykL4aCBsQSBw` |
| `clientProof(S43, {…T0, extensionVersion:"\ud800"})` = with `"�"` | `R0VmAdUMNhTHUdfs5BBGAC3D6awoJLUSZtbxt5oRu7c` |
| `clientProof("", T0)` | throws `DataError` "Zero-length key is not supported" |
| `isNonce("😀"×8)`, `isNonce("a"×15)` | `true`, `false` |
| `sameProof(42, p)`, `sameProof(p+"x", p)`, `sameProof(p, p)` | `false`, `false`, `true` |

Proposed `rust-anything record --inputs` JSONL for the `handshake.ts` unit. Format
`{"fn": name, "args": [...]}` (rust-anything 4.1.0 `record --help`). Five of the exports are async, so
confirm the recorder awaits Promises. `newNonce` is random: gate it on its shape only (22 chars,
`[A-Za-z0-9_-]`).

```jsonl
{"fn":"pairingSecret","args":["ABCD2345",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"}]}
{"fn":"pairingSecret","args":["",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"}]}
{"fn":"pairingSecret","args":["ZZZZZZZZZZZZZZZZZZZZZZZZZZ",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"ünïcödé-nonce-16+","serverNonce":"_____________________w"}]}
{"fn":"clientProof","args":["bHj22qBUKo2-HZgUPOTc6SiIHf8Y4m1UmHtixx8fEto",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"}]}
{"fn":"clientProof","args":["SSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS",{"protocolVersion":26.0,"manifestHash":null,"clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"}]}
{"fn":"clientProof","args":["SSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS",{"protocolVersion":1e21,"extensionVersion":["a",null,["b"]],"manifestHash":{"x":1},"clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"}]}
{"fn":"clientProof","args":["",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"}]}
{"fn":"serverProof","args":["bHj22qBUKo2-HZgUPOTc6SiIHf8Y4m1UmHtixx8fEto",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"},{"daemonVersion":"0.9.2","manifestHash":"1a2b3c4d","manifestInSync":true,"sealedSessionKey":"zR6r4OCmRJs9Cj_HswPDQBM1uXNLBjKZRw0WBCbkVJIHg6IkujXxZwy3FA"}]}
{"fn":"serverProof","args":["SSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"},{"daemonVersion":"0.9.2","manifestHash":"1a2b3c4d","manifestInSync":false}]}
{"fn":"sealSessionKey","args":["bHj22qBUKo2-HZgUPOTc6SiIHf8Y4m1UmHtixx8fEto",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"},"SSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS"]}
{"fn":"sealSessionKey","args":["bHj22qBUKo2-HZgUPOTc6SiIHf8Y4m1UmHtixx8fEto",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"},"kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk"]}
{"fn":"sealSessionKey","args":["bHj22qBUKo2-HZgUPOTc6SiIHf8Y4m1UmHtixx8fEto",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"},""]}
{"fn":"openSessionKey","args":["bHj22qBUKo2-HZgUPOTc6SiIHf8Y4m1UmHtixx8fEto",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"}," zR6r4OCmRJs9Cj/HswPDQBM1uXNLBjKZRw0WBCbkVJIHg6IkujXxZwy3FA"]}
{"fn":"openSessionKey","args":["bHj22qBUKo2-HZgUPOTc6SiIHf8Y4m1UmHtixx8fEto",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"},"zR6r4OCmRJs9Cj_HswPDQBM1uXNLBjKZRw0WBCbkVJIHg6IkujXxZwy3FA="]}
{"fn":"openSessionKey","args":["bHj22qBUKo2-HZgUPOTc6SiIHf8Y4m1UmHtixx8fEto",{"protocolVersion":26,"extensionVersion":"0.9.2","manifestHash":"1a2b3c4d","clientNonce":"AAAAAAAAAAAAAAAAAAAAAA","serverNonce":"_____________________w"},"A"]}
{"fn":"sameProof","args":[42,"1IrbKWjEXM59Shtx7gMpzLbuqQiaKAG8RJadvd1QtsU"]}
{"fn":"sameProof","args":["1IrbKWjEXM59Shtx7gMpzLbuqQiaKAG8RJadvd1QtsU","1IrbKWjEXM59Shtx7gMpzLbuqQiaKAG8RJadvd1QtsU"]}
{"fn":"sameProof","args":["1IrbKWjEXM59Shtx7gMpzLbuqQiaKAG8RJadvd1QtsV","1IrbKWjEXM59Shtx7gMpzLbuqQiaKAG8RJadvd1QtsU"]}
{"fn":"isNonce","args":["😀😀😀😀😀😀😀😀"]}
{"fn":"isNonce","args":["aaaaaaaaaaaaaaa"]}
{"fn":"isNonce","args":[1234567890123456]}
```

A lone-surrogate case such as `{"fn":"clientProof","args":["S…",{…,"extensionVersion":"\ud800"}]}`
works on the TS side but cannot be deserialized into a Rust `String` (section 7.5). Record it only if
the harness carries WTF-8 args; otherwise list it in `known-diffs.jsonl`. For the daemon's `greet` flow
(protocol suite), add frame-level cases: binary-frame `hello`, `"protocolVersion":26.0`, `1e400`,
`"extensionVersion":{"a":1}`, `"auth":"pair"` (→ `?.kind` is undefined → `bad-code`), and an
`auth.json` session with `"key":""` (→ 1011).

## Semantic traps

- The PBKDF2 output is used as **base64url text** for the HMAC key (`handshake.ts:38,96`).
- The keystream counter is **decimal text** in the MAC'd message, starting at 0 (`handshake.ts:115-116`).
- Every transcript ends in a separator field. The client message has a trailing `|`, and the server
  claims are four more `|` fields (`handshake.ts:101-109,58-63`).
- The daemon proves the **extension's** `protocolVersion` (`daemon.ts:358-361`) and leaves the welcome's
  own `protocolVersion` unauthenticated (`daemon.ts:569`).
- Untyped hello fields are coerced through `Array.prototype.join`/`ToString`, and the raw value is
  persisted (section 2.5).
- WebCrypto refuses a zero-length HMAC key (DataError, so 1011). RustCrypto accepts it.
- `isNonce` and `sameProof` measure **UTF-16** length.
- serde_json rejects lone surrogates and out-of-range numbers that `JSON.parse` takes.
  `serde_json::Number` prints floats differently from JS.
- TextEncoder hashes lone surrogates as EF BF BD. TextDecoder strips a BOM and `from_utf8_lossy` does not.
- `atob` is forgiving in ways no `base64` preset matches exactly (whitespace, mixed alphabet, trailing
  bits allowed, padding only when it completes a quad).
- Binary WebSocket frames are accepted for the JSON handshake (`daemon.ts:1145`).
- The `Bearer` strip uses JS `\s` over Latin-1-decoded header bytes, so 0xA0 counts as whitespace.
- `rand`'s default `choose`/`random_range` is (slightly) biased. Node's `randomInt` is not.

## Recommendation

1. Implement the handshake as a small `vexdom-handshake` module on `pbkdf2 0.13.0`, `hmac 0.13.0`,
   `sha2 0.11.1`, `subtle 2.6.1`, `base64 0.23.1` and `getrandom 0.4.3`, using the sketch in section 6
   unchanged. Keep `Transcript` fields as the JS strings, built by a shared
   `js_to_string(&serde_json::Value)` that uses `ryu-js 1.0.4`. Refuse an empty secret with an error
   that maps to close 1011.
2. Parse `hello`/`prove` as `serde_json::Value`, not a typed struct, and accept text or binary frames
   during the handshake. Decide explicitly, and record in `known-diffs.jsonl` if not copied, the lone-
   surrogate and `1e400` cases. The cheapest faithful option is a lossy pre-pass (lone surrogate →
   U+FFFD) for crypto, with the JS string kept for persistence.
3. Port Node's `randomInt` sampler as written (section 5). Do not use default-feature `rand`
   `choose` for codes.
4. Gate with the vectors in section 8, recorded through `rust-anything record --inputs`, plus
   protocol-suite cases for frame order, close codes and the exact `welcome` key order.
5. Do not port `openSessionKey`/`decode` into the daemon. If `ext-core` takes them later, include the
   forgiving decoder and the BOM strip from sections 2.3 and 2.4, and record them against TextDecoder
   first: the BOM-strip claim is from the spec and was not observed.
6. Any new `welcome` field (ADR-018's transport choice) must be appended to the `serverProof` claims
   join, byte-identically in TS and Rust, or it stays unauthenticated like `protocolVersion`.
