# JS regex and UTF-16 semantics: secrets, guardrails, manifest hash

Scope: (a) every regex in `packages/extension/src/lib/secrets/{detect,shapes,seal}.ts` and `src/daemon/guardrails/*.ts`,
plus the UTF-16 lengths and offsets in those files; (b) `hashManifest` in `packages/extension/src/lib/actions/manifest.ts`.
Bar: ADR-008 (`DECISIONS.md:68-73`).

Evidence: JS runs used Node v26.13.1 (V8 14.6.202.34, Unicode 18.0 per `process.versions`) on copies
of `detect.ts`/`shapes.ts`, with `looksLikeCardNumber` stubbed to `true` because no probe contains a
card-length digit run. Rust runs used regex 1.13.2, fancy-regex 0.20.2, serde_json 1.0.154
(`preserve_order`) and ryu-js 1.0.4 on rustc 1.100 (std Unicode 18.0.0), in a scratch crate outside the
repo. Every Rust spelling below was compiled, and the ones marked "verified" were run against the JS
result.

## 1. JS without `u` vs Rust, construct by construct

No pattern in scope uses the `u`, `v`, `y` (sticky), `s`, `m` or `d` flags (section 2). So every
pattern runs on **UTF-16 code units**, with the non-Unicode rules below.

| Construct | JS, no `u` flag | regex 1.13.2 default | Rust spelling with JS meaning |
|---|---|---|---|
| `\s` | WhiteSpace ∪ LineTerminator ([ES2027 §22.2.2.9 `CharacterClassEscape :: s`](https://tc39.es/ecma262/multipage/text-processing.html#sec-compiletocharset)): TAB VT FF ZWNBSP(U+FEFF) + Zs ([§12.2 Table 31](https://tc39.es/ecma262/multipage/ecmascript-language-lexical-grammar.html#sec-white-space)) + LF CR LS PS ([§12.3 Table 32](https://tc39.es/ecma262/multipage/ecmascript-language-lexical-grammar.html#sec-line-terminators)). Enumerated in Node: `9 a b c d 20 a0 1680 2000-200a 2028 2029 202f 205f 3000 feff` | `\p{White_Space}` (`regex-1.13.2/src/lib.rs:928`). It **adds U+0085 and drops U+FEFF** (observed) | `[{WS}]` with `WS` below. `(?-u:\s)` is `[\t\n\v\f\r ]` (`lib.rs:572,950`) and loses U+00A0 and the rest of Zs |
| `\d` | `0-9` only (§22.2.2.9 `:: d`) | `\p{Nd}` (`lib.rs:926`); matches U+0663 (observed) | `[0-9]` |
| `\w` | ASCII word characters. Extra characters only when a `u`/`v` flag *and* `i` are set ([§22.2.2.9.3 WordCharacters](https://tc39.es/ecma262/multipage/text-processing.html#sec-wordcharacters)) | Unicode word | `[A-Za-z0-9_]` or `(?-u:\w)` (`lib.rs:572`). Not used in scope |
| `\b` | `IsWordChar` on the code unit on each side, using WordCharacters ([§22.2.2.4.1](https://tc39.es/ecma262/multipage/text-processing.html#sec-runtime-semantics-iswordchar-abstract-operation)) | Unicode boundary (`lib.rs:574`); `é` counts as word, so there is no boundary in `éeyJ` (observed) | `(?-u:\b)` (regex crate). fancy-regex rejects inline `u` changes (`fancy-regex-0.20.2/src/lib.rs:770-777`) |
| `.` | all code units except LineTerminator ([§22.2.2.7 `Atom :: .`](https://tc39.es/ecma262/multipage/text-processing.html#sec-compileatom), [§22.2.2.9.4 AllCharacters](https://tc39.es/ecma262/multipage/text-processing.html#sec-allcharacters)) | everything except `\n` (`lib.rs:669`); matches `\r` and U+2028 (observed) | `DOT = [^\n\r\u{2028}\u{2029}]` |
| `i` | `Canonicalize` = toUppercase, kept only if the result is a single code unit, and a **non-ASCII char never maps to ASCII** ([§22.2.2.7.3](https://tc39.es/ecma262/multipage/text-processing.html#sec-runtime-semantics-canonicalize-ch) step 9). So ASCII letters match only their ASCII partner | Unicode simple case folding (`lib.rs:580`). `(?i)k` matches U+212A KELVIN SIGN and `(?i)s` matches U+017F LONG S (observed). These are the only two non-ASCII→ASCII simple folds in [CaseFolding-18.0.0](https://www.unicode.org/Public/UCD/latest/ucd/CaseFolding.txt) (`017F; C; 0073`, `212A; C; 006B`) | regex crate: `(?i-u:word)`. fancy-regex: `[Ww][Oo][Rr][Dd]` (helper `ci()` below) |
| `^` `$` (no `m`) | start/end of input | start/end of haystack (`lib.rs:756`); `^a$` does not match `"a\n"` (observed) | unchanged |
| counted `{n,m}`, `+`, `*` | counts **code units**. An astral char is 2 and each half matches any negated class (AllCharacters is code units without `u`) | counts code points ("the fundamental atom of matching is a single codepoint", `lib.rs:558-561`) | identical only on BMP text. See section 4 |
| match offsets | code-unit indices (`RegExpBuiltinExec`, [§22.2.7.2](https://tc39.es/ecma262/multipage/text-processing.html#sec-regexpbuiltinexec)) | UTF-8 byte offsets (`lib.rs:583`) | convert at the boundary (section 3) |
| look-behind / look-ahead | supported | **unsupported**: "look-around, including look-ahead and look-behind, is not supported" (`lib.rs:1-8`; observed error) | fancy-regex, a manual retry loop, or a consumed terminator (per pattern) |
| backreference | supported | unsupported (`lib.rs:5`) | hand-written code |
| match choice | backtracking: first alternative or longest greedy run that succeeds | `MatchKind::LeftmostFirst`, "the match semantics used by most backtracking engines" (`regex-automata-0.4.19/src/util/search.rs:1576-1579`) | same match and captures for patterns without look-around |
| iteration | `matchAll` clones the regex, so there is no shared `lastIndex` ([§22.2.6.9](https://tc39.es/ecma262/multipage/text-processing.html#sec-regexp-prototype-%symbol.matchall%)). After an empty match it advances one code unit ([§22.2.7.3](https://tc39.es/ecma262/multipage/text-processing.html#sec-advancestringindex)) | `find_iter`/`captures_iter` | no pattern in scope can match empty, so the empty-match rules never apply |
| `replace(re, fn)` | the callback's return value is inserted literally ([§22.2.6.11](https://tc39.es/ecma262/multipage/text-processing.html#sec-regexp.prototype-%symbol.replace%)) | closure `Replacer` pushes literally (`regex-1.13.2/src/regex/string.rs:2599-2606`) | `replace_all(&text, \|c: &Captures\| …)` |

Unicode versions: V8 reports Unicode 18.0, Rust std reports `(18, 0, 0)`, and regex-syntax 0.8.12's tables
are 16.0.0 (`regex-syntax-0.8.12/src/unicode_tables/perl_word.rs:5`). The spellings below use no Unicode
property classes, so the table version does not matter.

```rust
/// JS `\s` as a class body: WhiteSpace + LineTerminator (ES2027 §22.2.2.9, §12.2, §12.3).
const WS: &str = r"\t\n\x0B\x0C\r \u{A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}";
/// JS `.` without the `s` flag.
const DOT: &str = r"[^\n\r\u{2028}\u{2029}]";
/// JS `/i` without `/u` on an ASCII literal, for fancy-regex (which rejects `(?i-u:…)`).
fn ci(word: &str) -> String {
    word.chars().map(|c| if c.is_ascii_alphabetic() {
        format!("[{}{}]", c.to_ascii_uppercase(), c.to_ascii_lowercase())
    } else { regex::escape(&c.to_string()) }).collect()
}
```

## 2. Inventory

Sources: a scan for regex literals and `new RegExp` in every file in scope. `decide.ts`, `fence.ts`,
`index.ts`, `policy.ts`, `secrets.ts` and `settings.ts` contain none. `fence.ts:47` uses
`split`/`join`.

| # | Site | Flags | LB | LA | BR | Constructs that need care | Engine |
|---|---|---|---|---|---|---|---|
| 1 | `detect.ts:30` CANDIDATE | g | yes | yes | – | backtracks `={0,2}` into the look-ahead: run + `"==A"` matches run + `"="` (observed, 33 chars) | **fancy-regex**, same text (verified) |
| 2 | `detect.ts:31` UUID | i | – | – | – | `i` on hex only (no non-ASCII fold partner) | regex |
| 3 | `detect.ts:42` DATA_URL | g | – | – | – | `\b`, `\s`, `[^\s;,]{0,80}` counts code units | regex + view (section 4) |
| 4 | `detect.ts:99` `/^[0-9a-f]+$/i` | i | – | – | – | – | code: `bytes().all(u8::is_ascii_hexdigit)` |
| 5 | `detect.ts:100` `/[a-z]/` `/[A-Z]/` `/[0-9]/` | – | – | – | – | – | code: `bytes().any(...)` |
| 6 | `detect.ts:105` `/[^A-Za-z]/g` (replace) | g | – | – | – | – | code: `filter(char::is_ascii_alphabetic)` |
| 7 | `shapes.ts:104` `/[^a-z0-9]/g` after `toLowerCase()` | g | – | – | – | full Unicode lowercase first (`İ`→`i̇`, the same in both, observed) | code |
| 8 | `shapes.ts:121-122` `labelled()` ×3 (`:183-185`) | gi | yes | – | – | `i` on words with k/s, `\s`, `{4,400}` negated classes | **fancy-regex** with `ci()` (verified), or regex + retry loop (verified) |
| 9 | `shapes.ts:132-133` `prose()` ×2 (`:187-188`) | gi | yes | – | – | same, plus `[^\s"'<>]{3,399}[^…]` | **fancy-regex** with `ci()` (verified) |
| 10 | `shapes.ts:140` CREDENTIAL_SIGNAL | – | – | – | – | `\d` | regex |
| 11 | `shapes.ts:150-151` PLACEHOLDER | i | – | – | – | `i` (blank, sample, test, false…), `.`, `\s` | regex (verified) |
| 12 | `shapes.ts:155` `/^(.)\1*$/` | – | – | – | **yes** | `.` is one code unit, so `"😀😀"` fails (observed) | code `same_unit()` (verified) |
| 13 | `shapes.ts:164` private-key | g | – | – | – | `\s` inside a class; `{0,8000}` over a BMP-only class | regex |
| 14 | `shapes.ts:166-178` jwt, anthropic, openai, google, aws, github-pat, github-token, slack, stripe, npm, gitlab, sendgrid | g | – | – | – | `\b` (google, aws and npm on both ends) | regex with `(?-u:\b)` (jwt verified) |
| 15 | `shapes.ts:180` basic-auth | g | – | – | – | `\b`, `\s`, `{1,64}` `{3,128}` negated classes | regex + view |
| 16 | `shapes.ts:181` cookie-header | gi | – | – | – | `i` (cookie, set), `[^\r\n]{4,4000}` | regex + view (verified) |
| 17 | `shapes.ts:190` card | g | – | – | – | `\b`, `\d` | regex |
| 18 | `seal.ts:24` ANY_HANDLE (`matchAll` at `:65,194`, `replace(fn)` at `:164`) | g | – | – | – | none | regex, same text |
| 19 | `scope.ts:17` HOST_IN_TEXT | gi | – | yes | – | `i` on `[a-z]`, `\s` | regex with a consumed terminator (verified), or fancy-regex (verified) |
| 20 | `scope.ts:94` `/\.$/` `/^\*\./` `/^www\./` | – | – | – | – | one replacement each (no `g`) | code: `strip_suffix`/`strip_prefix` |
| 21 | `scope.ts:95` `/[^a-z0-9.\-[\]:]/` | – | – | – | – | – | code: `chars().all(...)` |
| 22 | `spawn.ts:90-101,104` FORBIDDEN (13) | i on 11 | – | – | – | `i` (danger**ously**, **s**andbox…) | regex `(?i-u)` (verified on 10 args) |
| 23 | `spawn.ts:102-103` `sandbox_mode=(?!"read-only"$)`, `approval_policy=(?!"never"$)` | i | – | **neg** | – | the look-ahead is case-insensitive too | code `forbidden_kv()` |
| 24 | `spawn.ts:193-194` SECRET_WORD | i | – | – | – | `i` (KEY, TOKEN, COOKIE; SECRET, SESSION…) | regex `(?i-u)` |

Adjacent and out of scope: `packages/extension/src/lib/recordings/events.ts:71-72` (`looksLikeCardNumber`, the card
validator: `/[\s-]/g`, `/^\d{13,19}$/`, so JS `\s` and ASCII `\d`); `daemon.ts:307` `/^Bearer\s+/i`
(handshake-crypto.md §4.5); `protocol.ts:356` `INSTALL_ID = /^[A-Za-z0-9_-]{16,64}$/` (ASCII, regex
as-is).

### Rust spellings (every one compiled; "verified" rows also run against the JS result)

```rust
// --- regex 1.13.2 ---------------------------------------------------------------------------
let uuid        = r"^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$";
let data_url    = format!(r"(?-u:\b)data:[^{WS};,]{{0,80}};base64,[A-Za-z0-9+/=]+");
let cred_signal = r"[0-9]|[!@#$%^&*()_+=\[\]{}|\\<>~/]|[a-z][A-Z]";
let placeholder = format!(r"^(?:(?i-u:null|nil|none|true|false|undefined|n/?a|empty|blank|test|demo|example|sample|changeme|hidden|redacted)|(?i-u:your)[\-_{WS}]{DOT}*|(?i-u:my)[\-_{WS}]{DOT}*|(?i-u:x){{3,}}|\*+|•+|\.{{3,}}|…+|-+|_+|\[[^\]]*\]|<[^>]*>|\{{\{{{DOT}*\}}\}}|\$\{{{DOT}*\}})$");
let private_key = format!(r"-----BEGIN(?:[A-Z ]{{0,32}})PRIVATE KEY-----[A-Za-z0-9+/={WS}]{{0,8000}}-----END(?:[A-Z ]{{0,32}})PRIVATE KEY-----");
let jwt         = r"(?-u:\b)eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}";
let anthropic   = r"(?-u:\b)sk-ant-[A-Za-z0-9_-]{16,}";
let openai      = r"(?-u:\b)sk-(?:proj-|svcacct-|admin-)?[A-Za-z0-9_-]{20,}";
let google      = r"(?-u:\b)AIza[0-9A-Za-z_-]{35}(?-u:\b)";
let aws         = r"(?-u:\b)(?:AKIA|ASIA|AIDA|AROA|AGPA|ANPA)[0-9A-Z]{16}(?-u:\b)";
let github_pat  = r"(?-u:\b)github_pat_[A-Za-z0-9_]{40,}";
let github_tok  = r"(?-u:\b)gh[pousr]_[A-Za-z0-9]{30,}";
let slack       = r"(?-u:\b)xox[abposr]-[A-Za-z0-9-]{10,}";
let stripe      = r"(?-u:\b)[rs]k_(?:live|test)_[A-Za-z0-9]{16,}";
let npm         = r"(?-u:\b)npm_[A-Za-z0-9]{36}(?-u:\b)";
let gitlab      = r"(?-u:\b)glpat-[A-Za-z0-9_-]{20,}";
let sendgrid    = r"(?-u:\b)SG\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}";
let basic_auth  = format!(r"(?-u:\b)https?://[^{WS}/:@]{{1,64}}:([^{WS}/@]{{3,128}})@");
let cookie      = r"(?:^|\n)[ \t]*(?:(?i-u:set-))?(?i-u:cookie)[ \t]*:[ \t]*([^\r\n]{4,4000})";
let card        = r"(?-u:\b)[0-9](?:[ \-]?[0-9]){12,18}(?-u:\b)";
let any_handle  = r"⟦([a-z-]+):([0-9a-z]+)(?:@([A-Za-z0-9._:\[\]-]{1,255}))?#([0-9a-f]{6,32})⟧";
// HOST_IN_TEXT: the look-ahead becomes a consumed terminator. It is equivalent because a match can
// only start with [A-Za-z0-9] or h/H, and no terminator char is one of those, so consuming it never
// hides the start of the next match.
let host_in_text = format!(r#"(?i-u:https?://)?((?:[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z]{{2,24}})(?:[/{WS},;:!?)"'\]]|$)"#);
let forbidden   = r"(?i-u)^(?:--dangerously|--yolo$|--force$|--auto-approve$|--always-approve$|--full-auto$|--no-sandbox$|--allow-all|--sandbox=?(?:workspace-write|danger-full-access|off)$|--permission-mode=?(?:bypassPermissions|acceptEdits)$)|(?i-u:danger-full-access)|^(?-i:-y|-f)$";
let secret_word = r"(?i-u)(?:^|_)(?:KEY|KEYS|TOKEN|TOKENS|SECRET|SECRETS|PASSWORD|PASSWD|CREDENTIAL|CREDENTIALS|AUTH|SESSION|COOKIE|PRIVATE|SIGNATURE)(?:$|_)";

// --- fancy-regex 0.20.2 (look-around); no inline `u`, so case is spelled with ci() --------------
let candidate = r"(?<![A-Za-z0-9+/_=\-])[A-Za-z0-9+/_\-]{32,4096}={0,2}(?![A-Za-z0-9+/_\-])";
fn inline(words: &[&[&str]]) -> String {                       // shapes.ts:87
    let sep = format!(r"[_\-{WS}]?");
    words.iter().map(|w| w.iter().map(|p| ci(p)).collect::<Vec<_>>().join(&sep)).collect::<Vec<_>>().join("|")
}
fn value() -> String {                                          // shapes.ts:119
    format!(r#"(?:{b}[{WS}]+|{ba}[{WS}]+|{t}[{WS}]+)?(?:"([^"\r\n]{{4,400}})"|'([^'\r\n]{{4,400}})'|([^{WS},;&"'<>{{}}\[\]]{{4,400}}))"#,
            b = ci("bearer"), ba = ci("basic"), t = ci("token"))
}
fn prose_value() -> String {                                    // shapes.ts:130
    format!(r#"(?:"([^"\r\n]{{4,400}})"|'([^'\r\n]{{4,400}})'|([^{WS}"'<>]{{3,399}}[^{WS}"'<>.,;:!?]))"#)
}
fn labelled(label: &str) -> String {                            // shapes.ts:121-122
    format!(r#"(?<![A-Za-z0-9])(?:{label})["']?[{WS}]*[:=][{WS}]*{v}"#, v = value())
}
fn prose(label: &str) -> String {                               // shapes.ts:132-133
    format!(r#"(?<![A-Za-z0-9])(?:{label})[{WS}]+(?:{is}|{are}|{was}|{will}[{WS}]+{be})[{WS}]*:?[{WS}]+{v}"#,
        is = ci("is"), are = ci("are"), was = ci("was"), will = ci("will"), be = ci("be"), v = prose_value())
}
// PASSWORD_WORDS / TOKEN_WORDS / COOKIE_WORDS copied from shapes.ts:49-85 as &[&[&str]].
let host_in_text_fancy = format!(r#"(?:[Hh][Tt][Tt][Pp][Ss]?://)?((?:[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z]{{2,24}})(?=[/{WS},;:!?)"'\]]|$)"#);

// --- hand-written ---------------------------------------------------------------------------
/// shapes.ts:155 `/^(.)\1*$/`: one code unit, not a LineTerminator, repeated over the whole string.
fn same_unit(v: &str) -> bool {
    let mut u = v.encode_utf16();
    match u.next() { Some(f) if ![0x0A, 0x0D, 0x2028, 0x2029].contains(&f) => u.all(|x| x == f), _ => false }
}
/// spawn.ts:102-103: `^key=(?!"allowed"$)` with /i, ASCII-only case folding (§22.2.2.7.3).
fn forbidden_kv(arg: &str, key: &str, allowed: &str) -> bool {
    arg.len() >= key.len() && arg.is_char_boundary(key.len())
        && arg[..key.len()].eq_ignore_ascii_case(key) && !arg[key.len()..].eq_ignore_ascii_case(allowed)
}
// forbidden_kv(a, "sandbox_mode=", "\"read-only\"") || forbidden_kv(a, "approval_policy=", "\"never\"")
```

Without fancy-regex, labelled and prose also work as **regex + retry loop**. The look-behind is a
single negative class at the very start, so: take the leftmost match of the pattern without it
(`captures_at(hay, at)`, which keeps the surrounding context, `regex-1.13.2/src/regex/string.rs:1193`).
If the character before `m.start()` is ASCII alphanumeric, search again from `m.start()` plus one
char. This gives the JS result on every probe in section 5 (`password:`, `xpassword:`,
`xpassword: a; password: …`, quoted, U+FEFF, NEL). CANDIDATE cannot be done this way: its look-ahead
interacts with backtracking (row 1). Keep fancy-regex there or write a scanner. fancy-regex's default
`backtrack_limit` is 1,000,000 and exceeding it is a runtime **error**
(`fancy-regex-0.20.2/src/lib.rs:682,811-817`), whereas V8 just keeps going. My estimate for CANDIDATE, not measured: at most about 4,096 × 3 backtrack
steps per start position, well under the limit.

Size: with default limits, `cookie` compiles in about 6 ms, `private_key` about 13 ms and the fancy
labelled patterns about 3 ms each. The default NFA limit is 10 MiB
(`regex-automata-0.4.19/src/meta/regex.rs:3223`).

## 3. UTF-16 lengths and offsets (non-regex) in scope

| Site | Expression | Unit | Rust |
|---|---|---|---|
| `detect.ts:59-63,67-78,127-129` | `match.index`, `match[0].length` → `Span.start/end`, `Range` | UTF-16 | Used only for overlap tests and `seal.ts` slicing; elsewhere only `kind` is read (`src/daemon/preferences.ts:79`). Byte offsets are fine inside Rust. **Bridged/recorded `findSecrets` output must be reported in UTF-16 units** (`Span` is public, `secrets/index.ts:15`) |
| `detect.ts:84-94` `secretIn` | `match[0].lastIndexOf(captured)` | UTF-16 | In every capturing pattern the group is followed by at most one char (closing quote or `@`) and is at least 3 long, so this equals the group's own start. Use `group.start()` |
| `detect.ts:97` | `value.length < 32` | UTF-16 | CANDIDATE values are ASCII, so `len()` |
| `detect.ts:116-125` `entropy` | `for…of` (code points), `/ value.length`, `Math.log2` | – | ASCII input. **Sum in first-occurrence order** (a `Map` iterates in insertion order, and float addition is not associative). `Math.log2` is "implementation-approximated" ([§21.3.2.24](https://tc39.es/ecma262/multipage/numbers-and-dates.html#sec-math.log2)). Node 26.13.1's V8 uses fdlibm/FreeBSD `e_log2` (`v8 14.6.202.34 src/base/ieee754.cc:1965`); V8 main has moved to llvm-libc (`src/base/ieee754.cc:216` on main). Rust `f64::log2` is the platform libm, so a ≤1-ulp difference could flip `>= 4.3` only for a value within about 1e-15 of it |
| `shapes.ts:143` | `value.length >= 6` | UTF-16 | `encode_utf16().count()` |
| `seal.ts:87` | `text.length < 4` | UTF-16 | `encode_utf16().count()`. Counter-example: `"⟦⟧x"` is 3 units but 7 bytes. TS returns it untouched, while a byte-length port in strict mode neutralizes it to `"⟨⟩x"` |
| `seal.ts:113,135` | `MIN_KEYED`: `node.length >= 4` | UTF-16 | `encode_utf16().count()`. `{password:"😀😀"}` is 4 units, so TS seals it; with a char count (2) it would not be |
| `seal.ts:185-190` `truncate` | `length - 4`, `slice(0,h)`, `slice(-t)` | UTF-16 | A non-zero `reveal` only comes from shapes that match ASCII-only text, so bytes equal units. Still, use UTF-16-aware helpers |
| `seal.ts:200-212` `neutralize` | per code unit, `inside(at)` over UTF-16 ranges | UTF-16 | Iterate `chars()` and keep the offset in the same unit as the immune ranges. Handles never split a pair |
| `seal.ts:143`, `:221` | `Object.entries` then `out[name] = …` | – | JS key order puts array-index keys first, ascending (section 6.2). **`"__proto__"` is dropped**: assignment calls the setter (observed: `{"b":1,"__proto__":{"x":1},"2":3}` → `{"2":3,"b":1}`) |
| `fence.ts:47` | `split(x).join(y)` ×3 | – | `.replace(OPEN,…).replace(CLOSE,…).replace(tag,"…")`. Both scan left to right without overlap ([§22.1.3.23](https://tc39.es/ecma262/multipage/text-processing.html#sec-string.prototype.split); [`str::match_indices`](https://doc.rust-lang.org/std/primitive.str.html#method.match_indices)) |
| `guardrails/secrets.ts:23` | `seq.toString(36)` | – | [Number::toString radix 36](https://tc39.es/ecma262/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-tostring), digits `0-9a-z`. Hand-written |
| `scope.ts:94` | `trim()`, `toLowerCase()` | – | `trim_matches(is_js_ws)`. **Not `str::trim`**: Rust std `is_whitespace` adds U+0085 and lacks U+FEFF (observed). JS `TrimString` uses WhiteSpace+LineTerminator ([§22.1.3.32.1](https://tc39.es/ecma262/multipage/text-processing.html#sec-trimstring)). `to_lowercase()` matches `toLowerCase` ([§22.1.3.28](https://tc39.es/ecma262/multipage/text-processing.html#sec-string.prototype.tolowercase)) on Unicode 18 (observed `İ`, `K`, final sigma `ΑΣ`→`ας`) |
| `scope.ts:82` | `split('.').pop()` | – | `rsplit('.').next()` |
| `scope.ts:188-191` | `Buffer.byteLength(url.search) + …(url.hash)` | UTF-8 | `search`/`hash` are `""` when the query or fragment is null **or empty**, otherwise `?`/`#` + value ([URL Standard, search and hash getters](https://url.spec.whatwg.org/#dom-url-search)) |
| `scope.ts:206`, `spawn.ts:225,230,247,266` | `length > 0`, `startsWith`, `toLowerCase() !== 'false'` | – | ASCII prefixes. `to_lowercase()` |

## 4. Exact UTF-16 counting on the regex crate: the "view" haystack

Counted quantifiers over negated classes (rows 3, 8, 9, 15, 16) count code units in JS. Observed:
`cookie: a😀b` gives JS the value `a😀b` (`a`, the pair and `b` make 4 units ≥ 4). Rust on the raw
string sees only 3 chars, backtracks `[ \t]*` and returns `" a😀b"`, a **different value**.

Every class in scope treats a surrogate half the same way: not `\s`, not a word char, not a line
terminator, no case partner, matched by every negated class and by `.`. Any BMP private-use char has
exactly the same membership. So matching over a view in which each astral char becomes two such
chars reproduces JS counting, and makes **view char index == UTF-16 index** (verified: cookie start
8, end 12 = JS).

```rust
/// Each char of `text` is one UTF-16 code unit of the original; `orig[i]` is the byte offset in the
/// original string of view char i (an astral char's two halves share its offset).
struct View { text: String, orig: Vec<usize> }
fn view(s: &str) -> Option<View> {
    if s.chars().all(|c| (c as u32) <= 0xFFFF) { return None; }        // fast path: match `s` itself
    let (mut text, mut orig) = (String::with_capacity(s.len() + 8), Vec::with_capacity(s.len()));
    for (at, c) in s.char_indices() {
        if (c as u32) > 0xFFFF { text.push('\u{E000}'); text.push('\u{E001}'); orig.extend([at, at]); }
        else { text.push(c); orig.push(at); }
    }
    Some(View { text, orig })
}
```

Map a view byte offset to a char index (the UTF-16 index), then through `orig` to the original byte
offset, and slice the value from the **original**. If a match boundary falls between `E000` and `E001`,
JS would have produced a lone surrogate in `value`, which a Rust `String` cannot hold. Only a
400+-unit bare value of astral chars can cause this (`{4,400}` truncation). Detect it and treat it as a
known diff, or carry strings as UTF-16 (`Vec<u16>`) through sealing. Lone surrogates already in the
input cannot reach Rust through serde_json at all (`serde_json-1.0.154/src/read.rs:951-1023`).

## 5. Probe results (observed)

| Input | JS | naive Rust | JS-faithful Rust |
|---|---|---|---|
| CANDIDATE on a 32-char mixed run + `"==A"` / `"==="` | `[0,33]` / `[0,34]` | regex crate: does not compile | fancy-regex: `(0,33)` / `(0,34)` |
| CANDIDATE on `x ` + `Ab1`×12 + ` y ` + `Q`×5000 | `[[2,38]]` | – | fancy-regex `[(2,38)]` |
| cookie-header on `cookie: a😀b` | value `a😀b`, start 8 | value `" a😀b"`, byte 7 | view: start 8, end 12 |
| labelled on `api \u212Aey: hunter22x` | no match | `(?i)` matches | `(?i-u:…)` no match |
| labelled on `pa\u017F\u017Fword: hunter22x` | no match | – | `ci()` fancy: none; retry loop: none |
| labelled on `password:\uFEFFhunter22x` | `hunter22x` @10 | `\s` misses FEFF, so the value includes it | `hunter22x` @ byte 12 (=10) |
| labelled on `password:\u0085hunter22x` | `\u0085hunter22x` @9 | `\s` eats NEL | `\u0085hunter22x` @9 |
| `\s` membership: U+FEFF / U+0085 / U+00A0 | yes / no / yes | no / yes / yes | yes / no / yes |
| jwt on `éeyJ….….…` | match @1 | `\b`: none | `(?-u:\b)`: byte 2 (=1) |
| `notAPlaceholder("blan\u212A")` | `true` | `(?i)` → `false` | `(?i-u)` → `true` |
| `notAPlaceholder("your-\u2028x")` | `true` | `.` → `false` | `DOT` → `true` |
| `notAPlaceholder("😀😀")`, `("aaaa")` | `true`, `false` | fancy `^(.)\1*$` → `false`, `false` | `same_unit` → `true`, `false` |
| HOST_IN_TEXT on `go to Example.COM, then https://a.b.co/x and foo.example.com.x or c.io)d.io` | `Example.COM, a.b.co, c.io, d.io` | – | both spellings identical |
| HOST_IN_TEXT on `read notes.txt and \u212A.io end.io` | `notes.txt, end.io` | – | both identical |
| FORBIDDEN over `--dangerously-skip --YOLO -y -Y -f --sandbox=off x-danger-full-access --permission-mode=acceptEdits --permission-mode=ACCEPTEDITS "--yolo\n"` | `T T T F T T T T T F` | – | identical |
| prose-password `Your temporary password is: Xy7!pass.` / labelled-token `Authorization: Bearer abc.def-123` | `Xy7!pass` / `abc.def-123` | – | identical |

Proposed `rust-anything record --inputs` lines for the secrets unit (`{"fn","args"}` JSONL; the
recorded `start`/`end` are UTF-16):

```jsonl
{"fn":"findSecrets","args":["cookie: a😀b"]}
{"fn":"findSecrets","args":["api \u212aey: hunter22x"]}
{"fn":"findSecrets","args":["pa\u017f\u017fword: hunter22x"]}
{"fn":"findSecrets","args":["password:\ufeffhunter22x"]}
{"fn":"findSecrets","args":["password:\u0085hunter22x"]}
{"fn":"findSecrets","args":["\u00e9eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghij"]}
{"fn":"findSecrets","args":["😀 sk-ant-abcdefghijklmnopqrstuv"]}
{"fn":"findSecrets","args":["aB3dE6gH9jK2mN5pQ8sT1vW4yZ7bC0eF==A"]}
{"fn":"findSecrets","args":["data:image/😀;base64,QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVphYmNkZWZnaGlqa2xtbm9wcXJzdHV2d3h5ejAxMjM0NTY3ODk="]}
{"fn":"findSecrets","args":["https://😀😀:pw😀@host/"]}
{"fn":"findSecrets","args":["token: 😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀"]}
{"fn":"findSecrets","args":["token: a😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀"]}
{"fn":"notAPlaceholder","args":["blan\u212a"]}
{"fn":"notAPlaceholder","args":["your-\u2028x"]}
{"fn":"notAPlaceholder","args":["😀😀"]}
{"fn":"looksLikeCredential","args":["a😀b1!"]}
{"fn":"kindForKey","args":["NEW-PASSWORDS"]}
{"fn":"entropy","args":["aB3dE6gH9jK2mN5pQ8sT1vW4yZ7bC0eFxQ"]}
{"fn":"caseFlips","args":["aB3dE6gH9jK2mN5pQ8sT1vW4yZ7bC0eF"]}
```

The two `token:` lines cut the bare value at 400 units (`{4,400}`). Observed: with 201 emoji the cut
lands on a pair boundary (`start 7, end 407`, last unit `de00`). With `a` + 200 emoji it **splits a
pair**: the last unit is a lone `d83d` (section 4). `looksLikeCredential("a😀b1!")` is `true` in JS
(6 units) and would be `false` with a char count (5). The ADR-008 normalizer must also treat the
sealing tag and seq ids (`guardrails/secrets.ts:20-23`) as tokens.

## 6. `hashManifest` (part b)

### 6.1 Algorithm (`manifest.ts:7-19`)
1. Copy the descriptors and sort by `name` with `a.name < b.name ? -1 : a.name > b.name ? 1 : 0`.
   String `<` compares **UTF-16 code units** lexicographically
   ([IsLessThan §7.2.12](https://tc39.es/ecma262/multipage/abstract-operations.html#sec-islessthan)), and the sort is
   **stable** for a consistent comparator ([§23.1.3.30.1](https://tc39.es/ecma262/multipage/indexed-collections.html#sec-sortindexedproperties)).
   Rust: `sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()))`. `slice::sort_by` is stable
   ([std](https://doc.rust-lang.org/std/primitive.slice.html#method.sort_by)). For ASCII names this is
   plain `str` order.
2. Map each descriptor to `[name, description, inputSchema]`, dropping other fields, then
   `canonical = JSON.stringify(array)` (section 6.2).
3. `hash = 0x811c9dc5`. For each **UTF-16 code unit** `u` of `canonical`
   ([`charCodeAt` §22.1.3.3](https://tc39.es/ecma262/multipage/text-processing.html#sec-string.prototype.charcodeat)):
   `hash ^= u` (`^=` is ToInt32 on both sides, so the bit pattern is unchanged), then
   `hash = Math.imul(hash, 0x01000193)` (product mod 2^32, [§21.3.2.20](https://tc39.es/ecma262/multipage/numbers-and-dates.html#sec-math.imul)).
   This is FNV-1a 32 over 16-bit units, **not** over bytes: a unit above 0xFF XORs 16 bits into the
   state. Observed: `[["x","😀",{}]]` hashes to `fe348926` over UTF-16 and `2a108182` over UTF-8 bytes.
4. Output `(hash >>> 0).toString(16).padStart(8, '0')`: unsigned
   ([§6.1.6.1.11](https://tc39.es/ecma262/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-unsignedRightShift)),
   lowercase hex ([§6.1.6.1.20](https://tc39.es/ecma262/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-tostring)),
   zero-padded to 8 ([StringPad §22.1.3.17.2](https://tc39.es/ecma262/multipage/text-processing.html#sec-stringpad)).
   Rust: `format!("{h:08x}")` with `u32::wrapping_mul`.

Where it matters: the daemon hashes its own bundled lists for `chromium` and `firefox`
(`daemon.ts:194-201`), and the extension hashes its own (`socket.ts:554`). `manifestInSync` and the
welcome's `manifestHash` depend on the two agreeing (`daemon.ts:443-448`). The daemon never hashes a
received list: `adoptExtensionManifest` stores it without hashing (`daemon.ts:709-718`). The schemas
come from zod's `z.toJSONSchema` with `$schema` and safe-integer bounds removed by `tidy`
(`packages/extension/src/lib/actions/registry.ts:123-149`). The Rust side therefore needs the same descriptor JSON,
recorded from TS.

### 6.2 JSON.stringify rules that reach the hash
- **Key order** = `[[OwnPropertyKeys]]`: array-index keys in ascending numeric order, then other
  strings in creation order ([OrdinaryOwnPropertyKeys §10.1.11.1](https://tc39.es/ecma262/multipage/ordinary-and-exotic-objects-behaviours.html#sec-ordinaryownpropertykeys),
  through EnumerableOwnProperties in SerializeJSONObject, [§25.5.4.5](https://tc39.es/ecma262/multipage/structured-data.html#sec-serializejsonobject)).
  An array index is a canonical numeric string with value 0…2^32−2
  ([§6.1.7](https://tc39.es/ecma262/multipage/ecmascript-data-types-and-values.html#sec-object-type)). Observed:
  `{"a":1,"-0":2,"4294967294":3,"4294967295":4,"1e3":5,"00":6,"7":7}` → keys
  `7, 4294967294, a, -0, 4294967295, 1e3, 00`. A duplicate key in `JSON.parse` input keeps its first
  position and takes the last value (observed `{"a":1,"b":2,"a":3}` → `{"a":3,"b":2}`).
- **Strings** ([QuoteJSONString §25.5.4.3](https://tc39.es/ecma262/multipage/structured-data.html#sec-quotejsonstring)):
  `\b \t \n \f \r \" \\` as short escapes. Other code points below 0x20 and **lone surrogates** become
  `\u` + 4 **lowercase** hex digits ([UnicodeEscape §25.5.4.4](https://tc39.es/ecma262/multipage/structured-data.html#sec-unicodeescape)).
  Everything else is raw, including `/`, DEL, U+2028/U+2029 and all non-ASCII (observed in the
  `nonAscii` vector).
- **Numbers**: finite → `Number::toString` (`26.0`→`26`, `-0`→`0`, `1e21`→`1e+21`,
  `1e20`→`100000000000000000000`, `123456789012345678901234567890`→`1.2345678901234568e+29`).
  Non-finite → `null` ([SerializeJSONProperty §25.5.4.2](https://tc39.es/ecma262/multipage/structured-data.html#sec-serializejsonproperty)).
- **undefined**: dropped as an object member, `null` as an array element
  ([SerializeJSONArray §25.5.4.6](https://tc39.es/ecma262/multipage/structured-data.html#sec-serializejsonarray)).
  A descriptor without `description` therefore hashes as `["x",null,…]` (observed `undef` vector).

### 6.3 serde_json 1.0.154 against those rules
- `preserve_order` keeps "the order of map keys in the input" (`Cargo.toml.orig:62-65`, IndexMap).
  For duplicates, IndexMap `insert` leaves "the key… in its place in the order", with the value
  updated (`indexmap-2.14.2/src/map.rs:439-441`), which matches JS (observed). It does **not** move
  array-index keys first (observed `intKeys`: `{"b":1,"2":2,…}`, hash `da760b8c` vs JS `8b5efb14`).
- Escaping: serde_json's `ESCAPE` table escapes exactly 0x00-0x1F, `"` and `\`, using `\b \t \n \f \r`
  and lowercase `\u00xx` (`serde_json-1.0.154/src/ser.rs:2135-2165,1785-1793`). That is identical to
  JS for any Rust `String` (observed).
- Numbers: the f64 writer is `zmij` (`ser.rs:1716-1723`). It prints `26.0`, `-0.0`, `1e+20`
  (observed), so `numbers` hashes to `735edc76` vs JS `d670b827`. Integers that parse as u64/i64 print
  the same as JS. Default float **parsing** is "best-effort precision". `float_roundtrip` gives
  correctly rounded f64s, which is what `JSON.parse` produces (`Cargo.toml.orig:67-74`).
  `arbitrary_precision` keeps the input text (`Cargo.toml.orig:76-81`), so `26.0` stays `26.0`, which
  is wrong for JS.
- Lone surrogates cannot be represented (parse error, `read.rs:951-1023`). Recursion is capped at a
  depth of 128 (`de.rs:63`). Neither matters for static descriptors.

The real descriptors happen to avoid every difference above. The chromium list from the local dist
build (`node src/daemon/dist/cli.js tools`, 52 tools, 61,299 UTF-16 units, 164 non-ASCII chars, no
astral, no integer-like keys, one float `0.35`) hashes to **`381a1f7e`** under TS, under plain
`serde_json::to_string` with `preserve_order`, and under the sketch below. That is luck, not
equivalence.

### 6.4 Rust sketch (verified on every vector in 6.5)

```rust
use serde_json::Value;          // features = ["preserve_order", "float_roundtrip"]
use std::fmt::Write;

fn array_index(k: &str) -> Option<u32> {                 // ES2027 §6.1.7
    if k == "0" { return Some(0); }
    if k.is_empty() || k.starts_with('0') || !k.bytes().all(|b| b.is_ascii_digit()) { return None; }
    k.parse::<u64>().ok().filter(|&n| n <= 4_294_967_294).map(|n| n as u32)
}

fn quote(s: &str, out: &mut String) {                    // §25.5.4.3 (no lone surrogates in a String)
    out.push('"');
    for c in s.chars() {
        match c {
            '\u{8}' => out.push_str("\\b"), '\t' => out.push_str("\\t"), '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"), '\r' => out.push_str("\\r"),
            '"' => out.push_str("\\\""), '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => { let _ = write!(out, "\\u{:04x}", c as u32); }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// JSON.stringify(v) for a value that came from JSON.parse (no undefined, functions, toJSON).
pub fn js_stringify(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            let f = n.as_f64().expect("no arbitrary_precision");
            if f.is_finite() { out.push_str(ryu_js::Buffer::new().format_finite(f)) } else { out.push_str("null") }
        }
        Value::String(s) => quote(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() { if i > 0 { out.push(',') } js_stringify(item, out) }
            out.push(']');
        }
        Value::Object(map) => {                          // §10.1.11.1
            let mut indexed: Vec<_> = map.iter().filter_map(|(k, v)| array_index(k).map(|i| (i, k, v))).collect();
            indexed.sort_by_key(|(i, _, _)| *i);
            let named = map.iter().filter(|(k, _)| array_index(k).is_none());
            out.push('{');
            for (n, (k, v)) in indexed.into_iter().map(|(_, k, v)| (k, v)).chain(named).enumerate() {
                if n > 0 { out.push(',') }
                quote(k, out); out.push(':'); js_stringify(v, out);
            }
            out.push('}');
        }
    }
}

pub fn fnv1a32_utf16(s: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for unit in s.encode_utf16() { h ^= u32::from(unit); h = h.wrapping_mul(0x0100_0193); }
    format!("{h:08x}")
}

/// manifest.ts:7-19. A missing description is `Value::Null` (undefined in an array → null).
pub fn hash_manifest(tools: &[(String, Value, Value)]) -> String {
    let mut sorted: Vec<_> = tools.iter().collect();
    sorted.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
    let mut c = String::from("[");
    for (i, (name, description, schema)) in sorted.into_iter().enumerate() {
        if i > 0 { c.push(',') }
        c.push('['); quote(name, &mut c); c.push(',');
        js_stringify(description, &mut c); c.push(',');
        js_stringify(schema, &mut c); c.push(']');
    }
    c.push(']');
    fnv1a32_utf16(&c)
}
```

`js_stringify` is also the serializer ADR-008 needs for any JSON the daemon re-emits after
`JSON.parse`-like handling, such as sealed results (section 3, `seal.ts:143`). The exceptions are
`__proto__` and lone surrogates.

### 6.5 Test vectors (Node v26.13.1 on an unmodified `manifest.ts`; Rust sketch identical)

| Case | Input `tools` (JSON) | Canonical (JSON-escaped) | Hash |
|---|---|---|---|
| empty | `[]` | `[]` | `741638a5` |
| one | `[{"name":"page.click","description":"Click","inputSchema":{"type":"object"}}]` | `[["page.click","Click",{"type":"object"}]]` | `bf51c7d9` |
| sorted | `[{"name":"b","description":"B","inputSchema":{}},{"name":"a","description":"A","inputSchema":{"properties":{"x":{"type":"string"}},"required":["x"]}}]` | `[["a","A",{"properties":…,"required":["x"]}],["b","B",{}]]` | `5a795870` |
| nonAscii | description `"café — “quotes” \u2028 \u007f \u0001 \t \"q\" \\ /"`, schema `null` | `\u2028`/DEL raw, `\u0001`, `\t`, `\"`, `\\`, `/` | `c6bfaa6c` |
| astral | description `"😀"`, schema `{}` | raw | `fe348926` |
| lone | description `"\ud800"` (TS only) | `\ud800` escaped | `67aceb74` |
| intKeys | schema `{"b":1,"2":2,"a":3,"1":4,"4294967295":5,"01":6}` | `{"1":4,"2":2,"b":1,"a":3,"4294967295":5,"01":6}` | `8b5efb14` |
| numbers | schema `{"a":26.0,"b":1e21,"c":1e-7,"d":0.1,"e":-0,"f":123456789012345678901234567890,"g":1.5e300,"h":2e-7}` | `{"a":26,"b":1e+21,"c":1e-7,"d":0.1,"e":0,"f":1.2345678901234568e+29,"g":1.5e+300,"h":2e-7}` | `d670b827` |
| undef | `[{"name":"x"}]` | `[["x",null,null]]` | `acdadd5d` |
| dupKey | schema text `{"a":1,"b":2,"a":3}` | `{"a":3,"b":2}` | `364c38b3` |
| chromium (local dist build) | `describeActions('chromium')` | 61,299 units | `381a1f7e` |

`record --inputs` lines (args are the `tools` array). Re-record `chromium`/`firefox` from HEAD: the
dist build used here was built 2026-11-25 15:06, and only `chromium` is reachable through
`cli.js tools` (`src/daemon/cli.ts:255-265`).

```jsonl
{"fn":"hashManifest","args":[[]]}
{"fn":"hashManifest","args":[[{"name":"page.click","description":"Click","inputSchema":{"type":"object"}}]]}
{"fn":"hashManifest","args":[[{"name":"b","description":"B","inputSchema":{}},{"name":"a","description":"A","inputSchema":{"properties":{"x":{"type":"string"}},"required":["x"]}}]]}
{"fn":"hashManifest","args":[[{"name":"x","description":"café — “quotes” \u2028 \u007f \u0001 \t \"q\" \\ /","inputSchema":null}]]}
{"fn":"hashManifest","args":[[{"name":"x","description":"😀","inputSchema":{}}]]}
{"fn":"hashManifest","args":[[{"name":"x","description":"","inputSchema":{"b":1,"2":2,"a":3,"1":4,"4294967295":5,"01":6}}]]}
{"fn":"hashManifest","args":[[{"name":"x","description":"","inputSchema":{"a":26.0,"b":1e21,"c":1e-7,"d":0.1,"e":-0,"f":123456789012345678901234567890,"g":1.5e300,"h":2e-7}}]]}
{"fn":"hashManifest","args":[[{"name":"x"}]]}
{"fn":"hashManifest","args":[[{"name":"b","description":"2"},{"name":"a","description":"1"},{"name":"b","description":"1"}]]}
{"fn":"hashManifest","args":[[{"name":"\uff21","description":""},{"name":"😀","description":""}]]}
```

The last two exercise sort stability and UTF-16 vs code-point order: U+FF21 sorts *after* `😀`
(D83D…) in UTF-16 but before it in code-point order.

## Semantic traps

- `\s` differs in two code points: JS has U+FEFF and not U+0085, Rust has the reverse. `str::trim`
  inherits the same difference.
- `\b` and `\d` are ASCII in JS and Unicode in Rust. `.` excludes `\r`, U+2028 and U+2029 in JS.
- `i` without `u`: ASCII letters never match KELVIN SIGN or LONG S in JS, but do under Rust `(?i)`.
  Labels and placeholders full of `k` and `s` are affected.
- fancy-regex cannot toggle Unicode inline, so `(?-u:\b)` and `(?i-u:…)` fail to compile there
  (`ChangingUnicodeModeUnsupported`). Spell case with classes.
- Counted repetitions and every offset are in UTF-16 units. Astral text changes *which value* is
  found, not just where (cookie probe).
- `.` in `/^(.)\1*$/` is one code unit, so an all-emoji string is not "one repeated char" in JS.
- CANDIDATE's look-ahead backtracks into `={0,2}`, so it cannot be emulated with a consumed suffix.
- fancy-regex's backtrack limit turns a pathological input into an `Err`, not a slow match.
- `entropy` sums in Map insertion order, and `Math.log2` is implementation-approximated (Node 26's
  V8 uses fdlibm).
- `sealValue` rebuilds objects by assignment: array-index keys move first and `__proto__` disappears.
- `hashManifest` is FNV-1a over UTF-16 code units of a JS-ordered, JS-number-formatted JSON string,
  sorted by UTF-16 order with a stable sort.
- serde_json differs on integer-like key order, f64 formatting (`26.0`, `-0.0`, `1e+20`), lone
  surrogates and best-effort float parsing, even though its escape table matches.
- `findSecrets`' public `Span` reports UTF-16 offsets. A bridged port must convert them back.

## Recommendation

1. Use **regex 1.13.2** for rows 2, 3, 10, 11, 13-18, 19, 22 and 24, spelled exactly as in section 2:
   the `WS`/`DOT` constants, `(?-u:\b)`, `[0-9]`, and `(?i-u:…)` for every case-insensitive literal.
   Never use bare `\s`, `\d`, `\b`, `.` or `(?i)`.
2. Use **fancy-regex 0.20.2** only for CANDIDATE (row 1). Labelled and prose (rows 8-9) can use
   fancy-regex with `ci()` or the regex + retry loop; prefer the loop, so all Unicode-off spelling
   stays in one engine. HOST_IN_TEXT (row 19) uses the regex crate with the consumed terminator.
3. Hand-write rows 4-7, 12, 20, 21 and 23 (`same_unit`, `forbidden_kv`, `is_js_ws` trim).
4. Run all secret shapes over the **UTF-16 view** (section 4). There is a fast path when the text has
   no astral chars. Keep offsets internal and convert `Span` to UTF-16 at the bridge. Treat a match that
   splits a surrogate pair as a known diff, unless sealing moves to `Vec<u16>` strings.
5. Use `encode_utf16().count()` for every `.length` threshold in section 3. Keep a first-seen-ordered
   counter for `entropy`. Port V8 14.6's fdlibm `log2` only if gating shows a flip.
6. For the manifest, embed the TS-recorded descriptor JSON and hash it with `hash_manifest` and
   `js_stringify` (section 6.4; serde_json with `preserve_order` + `float_roundtrip`, ryu-js 1.0.4).
   Gate on the section 6.5 vectors plus freshly recorded `chromium` and `firefox` hashes. Reuse
   `js_stringify` wherever ADR-008 requires TS-identical JSON output.
7. Record the section 5 JSONL through `rust-anything record --inputs` before porting, so the astral,
   BOM, NEL, KELVIN, LONG S and `é`-boundary cases are gated rather than assumed.
