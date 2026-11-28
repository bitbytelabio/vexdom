# So sánh tính năng: `vexdom` vs `../browsentic` & `../agent-browser`

> **Đã lỗi thời (2026-11-27):** phần lớn mục §3.1A, §3.1B và §3.2 đã được làm; xem [improvement-roadmap-2026-11.md](improvement-roadmap-2026-11.md).

- **Ngày đối chiếu:** 2026-11-26
- **Commit đối chiếu:**
  - `vexdom`: `d0490dc` (Rust Bridge/CLI/MCP + Cap'n Proto + `ext-core` wasm + GPUI `desktop/` + ADR-028)
  - `../browsentic`: `9c0892a` (v0.9.2 — commit fork gốc)
  - `../agent-browser`: `f0a9835` (v0.45.0)
- **Phạm vi:** chỉ xét tính năng liên quan đến **Claude Code** và **macOS**.
- **Loại trừ (ngoài plan):** 8 agent CLI khác trong `browsentic` (Codex, Antigravity, Mistral Vibe, Grok, Cursor CLI, Qwen, OpenCode, DeepSeek), Windows & Windows app, Linux packaging, Android phone mirror, iOS Simulator / Mobile Safari, cloud browser providers (Browserbase/Browserless/Browser Use/Kernel/AgentCore), và multi-engine headless (Lightpanda/Obscura).

---

## 1. Tổng quan nhanh

| Mảng tính năng | `vexdom` hiện tại | `browsentic` (0.9.2) | `agent-browser` (0.45.0) |
| --- | --- | --- | --- |
| Browser thật đã đăng nhập + Extension (Chromium & Firefox) | Có (64 tools Chromium / 55 Firefox) | Có (52 tools) | Chỉ attach CDP/BiDi, chủ yếu spawn headless |
| Side panel + Claude Code runner + Voice/Hands-free + A-Eye | Có đủ | Có đủ | Không (chỉ có TUI `chat`) |
| Captcha solver, Site maps, Recordings, Live tools (`injectCode`/`runCode`), Schedules, Themes | Có đủ | Có đủ | Không |
| Snapshot refs (`@eN`), delta tự động, markdown `read`, `batch`, `diff`, `state` | Có (ADR-025, ADR-028) | Không (`getPageInfo` cũ) | Có (đầy đủ hơn về cờ & CDP) |
| macOS Desktop App | GPUI MVP (3 tab cơ bản) | Swift App đầy đủ (5 tab + Settings + updater) | Không (dùng web dashboard) |
| CLI/MCP ergonomics, profiles, `doctor`, `upgrade`, `flow`, CDP tools | Một phần | Cơ bản | Đầy đủ |

---

## 2. Thiếu so với `../browsentic` (macOS + Claude Code)

Core Bridge + Extension của `vexdom` đã giữ đủ **52/52** page tools của `browsentic` ([`packages/extension/src/lib/actions/registry.ts`](../packages/extension/src/lib/actions/registry.ts#L70-L139)) cùng guardrails, secret sealing, native-host và macOS `launchd` quarantine workaround ([`crates/vexdom-daemon/src/quarantine.rs`](../crates/vexdom-daemon/src/quarantine.rs)).

Khoảng cách lớn nhất nằm ở **macOS Desktop App** và **cập nhật / phân phối**:

### 2.1 macOS Desktop App (`desktop/` GPUI vs Swift app của `browsentic`)

Hiện [`desktop/src/ui.rs`](../desktop/src/ui.rs#L51-L56) chỉ có 3 tab (`Overview`, `Browsers`, `Agent`) và các lệnh cơ bản ([`desktop/src/bridge.rs`](../desktop/src/bridge.rs#L19-L39)). So với [`../browsentic/docs/guide/mac-app.md`](../../browsentic/docs/guide/mac-app.md), còn thiếu:

- **First-run checks & 1-click setup:** màn hình kiểm tra ban đầu (Bridge, Browser, Claude Code) kèm nút cài/fix Claude Code khi thiếu.
- **Tab Settings (5 trang):**
  - *General:* chọn Light/Dark/System cho cửa sổ app (hiện chỉ tự theo OS), cấu hình tự chạy Bridge khi mở app / launch at login, **Browser theme** & **Guardrails** đồng bộ 2 chiều với extension, nút link `vexdom` vào `PATH` (`~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`), snippet copy lệnh cài MCP cho Claude Code, và nút **Uninstall…**.
  - *Skills:* danh sách skills (`bundled` / `user` / `uploaded`) + bộ lọc.
  - *Activity:* quản lý standing approvals (xoá theo site) + danh sách downloads đã capture.
  - *Logs:* xem live tail `~/.vexdom/daemon.log`.
  - *Agents:* thêm nút cài / `agent fix` cho Claude Code (hiện chỉ hiển thị status + chọn model).
- **Tab Browsers & Pairing:**
  - Nút **Get a pairing code** kèm đếm ngược thời hạn.
  - Đổi chế độ pairing (`auto` / `ask` / `code`) và nút **Unpair all**.
  - Nút **Load it unpacked instead** + cảnh báo **Reload needed** khi thư mục unpacked mới hơn bản browser đang load.
- **Overview, Menu bar & About:**
  - Nút **Restart Bridge** trong cửa sổ và menu bar ([`desktop/src/tray.rs`](../desktop/src/tray.rs#L40-L51) mới có Start/Stop).
  - Phím tắt chuyển tab (`⌘1`–`⌘5`).
  - Tab **About** (copy version các thành phần, nút **Report a bug** điền sẵn thông tin, link docs/release notes).
- **Auto-updater trong Mac app:** tự kiểm tra GitHub Release, tải, xác thực chữ ký, thay `Vexdom.app` và restart Bridge.

### 2.2 Packaging, Distribution & Self-Update (macOS)

- **`vexdom update` chưa tự tải bản mới:** [`app/src/cli/mod.rs`](../app/src/cli/mod.rs#L245-L249) hiện chỉ gọi `setup --no-pair --restart` (khởi động lại binary hiện tại và ghi đè thư mục unpacked cùng version), chưa kéo binary mới từ GitHub Releases (sau khi ADR-009 bỏ npm self-update).
- **Installer macOS cho `Vexdom.app` & `.dmg`:**
  - [`scripts/install.sh`](../scripts/install.sh#L1-L10) mới chỉ cài CLI Bridge vào `~/.local/share/vexdom`, chưa hỗ trợ cài `Vexdom.app` vào `/Applications` và tự gỡ `com.apple.quarantine` như `browsentic`.
  - [`.github/workflows/release.yml`](../.github/workflows/release.yml#L148-L155) đóng gói `Vexdom-*.zip` (chưa có `.dmg`, chưa tag release đầu tiên).
- **Store listings:** không có Chrome Web Store / Edge Add-ons (chủ đích ADR-010: dùng unpacked + signed unlisted Firefox `.xpi`).

### 2.3 Docs & Skills nội bộ

- [`docs/`](../docs/) vẫn là bản fork của Browsentic 0.9.2, chưa viết lại cho Rust Bridge, CLI, snapshot refs và GPUI app ([`README.md`](../README.md#L66-L68)).
- 4 nhóm tool mới thêm ở ADR-028 (`storage`, `navigation`, `input`, `code`) cùng `downloads`, `drag` chưa có skill markdown đi kèm (`skills: []` trong [`crates/vexdom-core/assets/tools.chromium.json`](../crates/vexdom-core/assets/tools.chromium.json)).

---

## 3. Thiếu so với `../agent-browser`

### 3.1 Làm được ngay trên kiến trúc hiện tại (không cần thêm `chrome.debugger`)

#### A. Snapshot, Frame & CLI ergonomics

- **`vexdom snapshot` CLI bị khóa `interactive: true`:** [`app/src/cli/page.rs`](../app/src/cli/page.rs#L868) hardcode `opts.set_interactive(true)`, không có cờ lấy full AX tree từ CLI (dù MCP `vexdom_snapshot` nhận `interactive: false`) và chưa có `--json` output có cấu trúc cho snapshot / page verbs.
- **`vexdom frame @eN` lỗi `UNSUPPORTED` trên Chrome:** [`packages/extension/src/lib/actions/page/frame-id.ts`](../packages/extension/src/lib/actions/page/frame-id.ts#L8-L18) gọi `runtime.getFrameId` (Chrome không hỗ trợ trong content script; [`HANDOFF.md`](../HANDOFF.md#L47)), dù `page.snapshot` đã có cơ chế gán `frameId` qua `postMessage` nonce ([`packages/extension/src/lib/bridge/snapshot-frames.ts`](../packages/extension/src/lib/bridge/snapshot-frames.ts)).
- **Pre-checks cho element actions:**
  - [`click-element.ts`](../packages/extension/src/lib/actions/page/click-element.ts#L79-L89) chưa kiểm tra phần tử bị che (`covered by <...>`) hay hidden/disabled/ngoài viewport trước khi click (chỉ `page.trustedClick` có check bị che).
  - [`fill-input.ts`](../packages/extension/src/lib/actions/page/fill-input.ts#L23-L44) chưa xử lý riêng `date`/`time`/`color`/`range` và chưa trả lỗi rõ cho `readonly`/`disabled`.
- **`vexdom read <url>` luôn navigate active tab:** [`app/src/cli/page.rs`](../app/src/cli/page.rs#L906-L914) chuyển tab hiện tại sang URL rồi mới convert DOM; chưa có chế độ fetch HTTP thuần không đụng tab (`Accept: text/markdown`, thử `.md`, tự tìm `llms.txt`, `--llms`, `--require-md`, `--raw`).
- **Một số cờ/sub-features CLI còn thiếu:**
  - `find ... --exact` ([`dom.ts`](../packages/extension/src/lib/actions/page/dom.ts#L150-L154) chỉ khớp substring với `text`).
  - `eval -b/--base64`, `eval --stdin` alias (hiện dùng `eval -`), và báo giá trị JS không serialize được sang JSON (`NaN`, `Infinity`, `-0`, `BigInt`, `undefined`).
  - `cookies set --curl <file>` (import từ JSON, Copy as cURL hoặc Cookie header).
  - `batch` nhận positional commands (`vexdom batch "open ..." "click @e1"`), `--json`, `--omit-command` ([`app/src/cli/page.rs`](../app/src/cli/page.rs#L1107-L1150) hiện chỉ đọc từng dòng từ stdin).
  - `tab new --label <name>` (đặt label cố định cho multi-tab workflow thay vì khớp title/URL).

#### B. MCP Server (dùng từ Claude Code CLI ngoài side panel)

- **Thiếu `--tools <profiles>` & pagination:** khi chạy `claude mcp add vexdom -- vexdom mcp`, server đẩy toàn bộ 62+ tools (~11,969 tokens, [`DECISIONS.md`](../DECISIONS.md#L398)) vì chế độ `on_demand` chỉ áp dụng cho side-panel run; thiếu profile lọc (`core`, `storage`, `debug`, …), phân trang `tools/list`, và annotations (`readOnlyHint`, `openWorldHint`) như [`../agent-browser/cli/src/mcp.rs`](../../agent-browser/cli/src/mcp.rs#L229-L300).
- **Thiếu MCP tools cho `diff` và `state`** (hiện mới có trên CLI theo ADR-028).
- **Thiếu flow `confirm` / `deny` từ CLI & MCP:** hiện mọi action dính guardrail `Confirm` khi gọi từ external CLI/MCP bị từ chối cứng (`BLOCKED`) trừ khi mở `unattended: "allow"` ([`crates/vexdom-core/src/guardrails/mod.rs`](../crates/vexdom-core/src/guardrails/mod.rs#L462-L477)). Chưa có cơ chế confirmation token (`vexdom confirm`/`deny`) hoặc đẩy hộp thoại duyệt sang GPUI desktop app.

#### C. Vận hành, Bảo mật, Workflows & Skills

- **`vexdom doctor` (`--fix`, `--json`)** và **`vexdom upgrade`**: chẩn đoán & tự sửa lockfile cũ, native host, lệch version extension, tình trạng Claude Code, tự nâng cấp binary.
- **Project-level config (`./vexdom.json`)**: cấu hình theo repo với allowlist key an toàn (`maxOutput`, `screenshotFormat`, `annotate`, v.v.).
- **WebMCP nâng cao:**
  - Tự động announce danh sách WebMCP tools khi navigate/snapshot kèm nhãn `[origin]` / `[cross-origin]` và lọc ký tự Unicode vô hình chống prompt injection ([`../agent-browser/cli/src/output.rs`](../../agent-browser/cli/src/output.rs#L576-L658)).
  - CLI `webmcp` hỗ trợ `--frame <frame-id>`, `--detach` / `result` / `cancel` ([`app/src/cli/page.rs`](../app/src/cli/page.rs#L693-L716)).
- **Saved Flows (`flow`):** ghi lại các API request first-party trong lúc thao tác và replay thẳng qua HTTP (fallback về page/UI khi drift) — nhanh và ít token hơn nhiều so với `recordings` (chỉ replay UI).
- **Auth vault (`auth save/login/list/show/delete`)** & **Plugin system (`plugin add/list/show/run`)** (kết nối credential provider bên ngoài, mở rộng custom command).
- **Specialized skills:** [`vexdom skills get`](../app/src/cli/mod.rs#L1163-L1190) hiện chỉ có `core` (+ 3 ref `snapshot-refs`, `read`, `batch`); chưa có các skill chuyên biệt như `dogfood`, `derive-client`, `webmcp-gen`, `slack`.

---

### 3.2 Cần `chrome.debugger` (đang out-of-scope theo ADR-028)

> **Lưu ý:** [`packages/extension/wxt.config.ts`](../packages/extension/wxt.config.ts#L34) **đã có sẵn quyền `debugger`** trên Chrome (đang dùng cho `trustedClick`, `findCaptcha`/`solveCaptcha`, `injectCode`, `startDiagnostics`/`readConsole`/`readNetwork`). Vì vậy các tính năng dưới đây **hoàn toàn làm được trên Chrome/Chromium** (đánh đổi thanh debugging bar, không chạy trên Firefox):

- **CLI verbs cho diagnostics có sẵn:** extension đã có `page.startDiagnostics`, `page.readConsole`, `page.readNetwork` qua MCP, nhưng **chưa nối ra CLI** (`vexdom console`, `vexdom errors`, `vexdom network requests/request`).
- **Network routing & HAR:** `network route` (chặn/mock response, lọc theo `--resource-type`), `network unroute`, `network har start/stop`.
- **Input cấp thấp & trusted keyboard:** `mouse move/down/up/wheel` (`--human`), và tổ hợp phím trusted (hiện `press Control+a` gửi synthetic event nên không bôi đen text thật, [`HANDOFF.md`](../HANDOFF.md#L64-L65)).
- **JS Dialogs:** `dialog status|accept|dismiss`, tự động accept `alert`/`beforeunload`.
- **Emulation (`set`):** `viewport`, `device`, `geo`, `offline`, `headers`, `credentials`, `media` (color scheme, reduced-motion).
- **Capture, Perf & DevTools sâu:**
  - `pdf` (`Page.printToPDF`).
  - `record` video WebM (+ cursor overlay + contact sheet qua screencast + `ffmpeg`).
  - `trace start/stop`, `profiler start/stop`, `vitals` (LCP/CLS/TTFB/FCP/INP).
  - `react` (`tree`, `inspect`, `renders`, `suspense`).
  - `a11y` audit (lưu ý `axe.min.js` có license MPL-2.0 nên ADR-003 không vendor).
  - `tab keep live|throttled|frozen` (chống Chrome throttle background tab).
  - `--init-script` / `removeinitscript`.

---

### 3.3 Khác mô hình kiến trúc (thường không cần port)

`vexdom` điều khiển trình duyệt thật đã đăng nhập qua extension + side panel, nên các tính năng dành cho headless/managed browser của `agent-browser` nhìn chung không cần port:

- Vòng đời browser độc lập: `open` launch headless, `close`, `connect --cdp`, `--auto-connect`, `--profile`, `--session` isolation nhiều browser song song, `--proxy`, `--ca-cert`, `--webgpu`.
- Quan sát headless: `stream` (WebSocket screencast), web `dashboard`, `inspect`, và TUI `chat`.

---

## 4. Gợi ý thứ tự ưu tiên (macOS + Claude Code)

1. **Hoàn thiện trải nghiệm macOS user (`desktop/` + distribution):**
   - Bổ sung Settings vào GPUI app (Guardrails, Browser theme, Logs, Approvals/Downloads, Skills, link `vexdom` vào `PATH`, copy lệnh MCP, Uninstall), nút Pairing code + Restart Bridge.
   - Làm tự cập nhật thật cho `vexdom update` / `upgrade` (GitHub Releases) và `Vexdom.app` updater; nâng cấp `scripts/install.sh` (cài `.app` + gỡ quarantine) hoặc build `.dmg`.
2. **Tối ưu sâu cho Claude Code (CLI & MCP):**
   - Thêm `vexdom mcp --tools <profiles>` (hoặc chế độ `on_demand` cho external MCP) để giảm ~12k token catalog khi dùng từ Claude Code CLI.
   - Sửa `vexdom frame` trên Chrome (tái sử dụng nonce đã có từ `snapshot-frames.ts`).
   - Mở cờ full tree + `--json` cho `vexdom snapshot`, thêm `vexdom console`/`errors`/`network` CLI gọi thẳng vào `diagnostics` sẵn có.
   - Thêm `vexdom doctor` và cơ chế duyệt guardrail cho CLI/MCP (`confirm`/`deny` hoặc đẩy prompt lên GPUI desktop app).
3. **Nâng độ tin cậy automation & tiết kiệm token:**
   - Check bị che (`covered by`) + hidden/disabled trong `page.clickElement` và `readonly`/special inputs trong `page.fillInput`.
   - Hỗ trợ HTTP-direct `vexdom read <url>` (`Accept: text/markdown` / `llms.txt`) không làm thay đổi active tab.
   - Viết lại `docs/` và bổ sung skill cho 4 nhóm tool mới (`storage`, `navigation`, `input`, `code`); cân nhắc `flow` (HTTP replay) cho tác vụ lặp.
