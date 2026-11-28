# Lộ trình cải thiện `vexdom`: token, độ trễ, độ tin cậy, markdown, CI

- **Ngày:** 2026-11-27
- **Commit gốc:** `3f9cb37` (v0.11.2)
- **Thay cho:** [feature-gap-comparison.md](feature-gap-comparison.md) (2026-11-26), phần lớn §3.1A, §3.1B
  và §3.2 của nó đã làm xong trước đợt này.

---

## 1. Phương pháp

Năm cuộc audit chỉ đọc mã, mỗi cuộc một mảng, cộng kiểm tra tay các điểm quan trọng:

| Audit | Phạm vi |
| --- | --- |
| SnapshotTokens | Cây snapshot, delta, câu trả lời của `act`, markdown, catalog MCP, bench token |
| LatencyPath | Đường đi của một `act` từ MCP tới content script và ngược lại |
| ActionReliability | Click/hover/fill/type/submit, shadow DOM, điều hướng, ref cũ |
| CodeHealth | CI, chuỗi cung ứng, coverage, kích thước wasm, bench |
| CompetitorScan | Playwright MCP, chrome-devtools-mcp, agent-browser, browser-use, Stagehand, Claude in Chrome, BrowserMCP, Atlas, Nanobrowser, BrowserOS, WebMCP, MCP 2026-07-28 |

Số token theo cách của `bench/tokens/count.mjs`: `ceil(chars / 4)`. Bench offline chạy lại trên
cùng các wire snapshot đã chụp (`bench/tokens/golden-after/pages`), nên so sánh trước/sau là cùng
nội dung trang.

---

## 2. Phát hiện theo mảng

### 2.1 Token (A)

| # | Phát hiện | Bằng chứng |
| --- | --- | --- |
| A1 | Cây Hacker News 5.053 token, bị cắt ở 20.000/26.163 ký tự; markdown cùng trang chỉ 1.027 | `bench/tokens/baseline.md` (trước đợt này) |
| A2 | Ô bảng luôn có ref, kể cả ở chế độ interactive; tên ô là chữ của mọi con (ô đầu một hàng HN 4,7k ký tự) | `crates/vexdom-snapshot/src/snapshot.rs` `assign_refs` (`TABLE_CELL_ROLES`), `snapshot.ts` `canRef` |
| A3 | Tên của phần tử có ref không bị giới hạn độ dài | `snapshot.rs` `render_node` |
| A4 | Cây không có `find` hay `viewport`; chỉ "Ask for less" khi bị cắt | `app/src/mcp/host.rs` `cut_to` |
| A5 | Delta là một splice duy nhất giữa tiền tố và hậu tố chung: hai thay đổi xa nhau gửi lại mọi dòng ở giữa | `crates/vexdom-snapshot/src/delta.rs:104-145` |
| A6 | Câu trả lời `act` lặp lại `selector`, `bounds`, `tag` của phần tử (~60–80 token/lần) | `packages/extension/src/lib/actions/page/dom.ts` `describeElement`, `host.rs` `render` |
| A7 | `DELTA_ACTIONS` thiếu `scrollTo`, `hoverElement`, `waitFor`, `dragElement` | `crates/vexdom-daemon/src/page.rs:31-44` |
| A8 | Markdown bỏ URL của link, ô bảng dính nhau, gạch đầu dòng `-` lơ lửng, giữ nội dung ẩn | `crates/vexdom-daemon/src/markdown.rs`, `read-html.ts` |
| A9 | Không có trần kích thước catalog | `app/tests/mcp.rs` |

### 2.2 Độ trễ (B)

| # | Phát hiện | Bằng chứng |
| --- | --- | --- |
| B1 | Mỗi action đổi trang ngủ cố định 150 ms trước snapshot delta | `page.rs:29` `SETTLE`, `with_delta` |
| B2 | Batch N bước trả N lần settle + N delta trung gian | `host.rs` `batch` |
| B3 | `offer()` gọi `Control.describe` mỗi `act`; manifest 170 KB clone rồi parse lại | `host.rs:536`, `crates/vexdom-daemon/src/lib.rs:669` |
| B4 | `childFrames` chạy `executeScript(allFrames)` mỗi snapshot, kể cả trang không có iframe | `bridge/snapshot-frames.ts:58-67` |
| B5 | `read_agent_config()` đọc và parse file cấu hình 3 lần mỗi lời gọi | `crates/vexdom-core/src/config.rs:46-55` |
| B6 | `scrollTo` mặc định `smooth` (300–1.000 ms) | `actions/page/scroll-to.ts` |

### 2.3 Độ tin cậy (C)

| # | Phát hiện | Bằng chứng |
| --- | --- | --- |
| C1 | Click/hover/setChecked từ chối ngay khi phần tử chưa bật, chưa hiện hoặc bị che; không chờ | `click-element.ts` `assertActionable` |
| C2 | `elementAt` dùng `document.elementsFromPoint`, chỉ thấy shadow host: nút trong shadow root mở bị báo "covered" | `actions/page/pointer.ts:61` |
| C3 | Click làm trang điều hướng: delta có thể chụp tài liệu cũ hoặc rỗng | `page.rs` `with_delta`, `snapshot-frames.ts` |
| C4 | Sau khi tiêm lại content script, action đổi trang không gửi lại (dù "Receiving end does not exist" nghĩa là chưa chạy); ref của trang đã tải lại báo `TAB_UNREACHABLE` thay vì `STALE_REF` | `actions/client.ts` `RETRY_AFTER_HEAL` |
| C5 | contenteditable được điền bằng `textContent =`, làm hỏng Lexical/ProseMirror/Slate | `fill-input.ts:83-86`, `type-text.ts` `clearContent` |
| C6 | `submitForm` không nói khi validation của trình duyệt chặn form | `submit-form.ts` |
| C7 | Click bắn sự kiện ở tâm hộp, kể cả khi tâm nằm ngoài viewport | `click-element.ts` `dispatchClick` |

### 2.4 Kỹ thuật và CI (E)

| # | Phát hiện | Bằng chứng |
| --- | --- | --- |
| E1 | `paths-ignore: '**/*.md'` bỏ qua cả `SKILL.md` được nhúng vào binary và có golden test | `.github/workflows/ci.yml` |
| E2 | Build/lint Firefox chỉ chạy lúc release | `.github/workflows/release.yml` |
| E3 | Không có cargo-deny, Dependabot hay audit | repo |
| E4 | Release không chờ CI | `release.yml` |
| E5 | Không có ngân sách kích thước wasm (2.563.777 byte) | `packages/extension/scripts/copy-ext-core.mjs` |
| E6 | Ngưỡng coverage 37 / 1 / 12 % thấp xa so với thực tế | `packages/extension/vitest.coverage.ts` |

---

## 3. Đợt này đã làm

| Mảng | Thay đổi | Nơi |
| --- | --- | --- |
| A2 | Chế độ interactive không cấp ref cho ô bảng; control bên trong đứng thay chỗ. Content script bỏ luôn việc tính tên ô | `snapshot.rs` `assign_refs`, `snapshot.ts` `walk` |
| A3 | Tên phần tử có ref in tối đa 150 ký tự rồi `…`; ref map giữ tên đầy đủ cho recovery | `snapshot.rs` `MAX_REF_NAME` |
| A4 | `find` (dòng khớp + cha, không dời baseline delta) và `viewport` (chỉ phần trên màn hình + dòng đếm vùng bị bỏ); MCP, CLI `--find`/`--viewport`, completions | `find.rs`, `page.rs`, `host.rs`, `cli/page.rs` |
| A5 | Delta nhiều hunk: `@ lines a-b replaced by:`, `@ inserted after line n:`, `@ lines a-b removed` (Myers của `similar`, hạn 50 ms) | `delta.rs` |
| A6 | MCP bỏ `selector`, `bounds`, `tag` khỏi kết quả action phần tử; CLI và side panel giữ nguyên | `host.rs` `slim_echo` |
| A7 | Thêm hover, drag, scroll, waitForElement, waitFor vào `DELTA_ACTIONS` | `page.rs` |
| A8 | Markdown: link `[text](url tuyệt đối)` khi `urls`, hàng bảng `\| a \| b \|`, nối bullet lơ lửng, bỏ (best effort) nội dung ẩn bởi `display:none`, `visibility:hidden`, `content-visibility:hidden` khỏi HTML đọc; `href` trong HTML là địa chỉ trình duyệt đã resolve | `markdown.rs`, `read-html.ts`, `read_url.rs` |
| A9 | Trần catalog: 5.750 byte (tools/list + instructions, đo được ~5.660) | `app/tests/mcp.rs` `CATALOG_CEILING` |
| B1, C3 | Bỏ giấc ngủ 150 ms: snapshot sau action cho trang đang tải tối đa 500 ms, rồi chờ 50 ms không có thay đổi DOM, tối đa 500 ms; nếu tài liệu cũ bị huỷ giữa chừng thì chờ tải xong, lấy lại danh sách frame và đọc lại | `snapshot.ts` `domQuiet`, `snapshot-frames.ts`, `page.rs` `SETTLE_CAP_MS` |
| B2 | Batch chỉ chụp một delta, sau bước đổi trang cuối cùng (`_noDelta` cho các bước khác) | `host.rs` `batch`, `page.rs` `take_no_delta`, `control.rs` |
| B6 | `scrollTo` mặc định `instant` | `scroll-to.ts` |
| C1 | `waitActionable`: thử lại mỗi 50 ms tới `timeoutMs` (mặc định 2.000) cho click/hover/setChecked; phần tử rời trang báo `STALE_REF` | `click-element.ts` |
| C2 | `elementAt` đi vào shadow root mở; `assertUncovered` chấp nhận phần tử nằm trong shadow root đóng của chính phần tử trúng | `pointer.ts` |
| C4 | Sau khi tiêm lại: input có ref thì `STALE_REF`; action đổi trang (`PAGE_CHANGERS`) không gửi lại; action đọc gửi lại đúng một lần | `client.ts` |
| C5 | contenteditable dùng `execCommand('insertText' / 'delete')`, chỉ rơi về `textContent` khi trình duyệt từ chối | `fill-input.ts` `editByCommand`, `type-text.ts` |
| C6 | `submitForm` trả `submitted: false` và tối đa 5 trường `invalid` kèm lý do `ValidityState` (không kèm thông báo của trình duyệt, vì nó có thể trích giá trị đã điền) | `submit-form.ts` |
| C7 | Click bắn ở tâm phần nhìn thấy (`elementPoint`) | `click-element.ts` |
| E1–E4 | CI theo `paths:` (tài liệu bỏ qua, `crates/*/assets/**` luôn chạy), `workflow_call`; bước build + lint Firefox; job `deny` cho cả hai workspace; Dependabot hàng tuần; release cần CI xanh | `ci.yml`, `deny.toml`, `.github/dependabot.yml`, `release.yml` |
| E5 | Ngân sách wasm 2.821.000 byte (+10%) | `copy-ext-core.mjs` |
| E6 | Ngưỡng coverage lên 58 / 34 / 39 % (đo được 58,5 / 34,0 / 39,8) | `vitest.coverage.ts` |

### 3.1 Số đo trước / sau (bench offline, cùng trang)

Từ `bench/tokens/baseline.md` sau `cargo test -p vexdom --test mcp_bench -- --ignored` và
`bun count.mjs --after`:

| Mục | Trước (token) | Sau (token) | Đổi |
| --- | ---: | ---: | ---: |
| hacker-news: snapshot | 5.053 (bị cắt) | 3.388 (không cắt) | −33% |
| hacker-news: snapshot as:markdown | 1.027 | 1.157 | +13% (hàng bảng có `\|`) |
| wikipedia-rust: snapshot | 5.054 | 5.058 | ~0 (vẫn chạm trần 20.000 ký tự; dùng `find`/`viewport`) |
| docs-rs-tokio: snapshot as:markdown | 4.763 | 4.758 | ~0 |
| catalog: tools/list | 696 | 829 | +19% (`find`, `viewport`, mô tả) |
| catalog: server instructions | 543 | 579 | +7% |

Độ trễ: giấc ngủ cố định 150 ms mỗi action đổi trang đã bỏ. Trên trang tĩnh, snapshot sau action
trả sau ~50 ms im lặng DOM; batch N bước bớt N−1 lần snapshot. Chưa đo trên Chrome thật.

### 3.2 Lệch so với kế hoạch

- `submitForm` tìm trường lỗi bằng `form.elements` + `validity.valid` thay vì `:invalid`
  (happy-dom không hỗ trợ selector đó; kết quả như nhau trên Chrome).
- `assertUncovered` chấp nhận trường hợp shadow root đóng khi `hit` chứa `el` trong cây composed
  nhưng **không** chứa trong light DOM, chặt hơn `!hit.shadowRoot` (điều kiện đó cũng đúng với mọi
  phần tử cha thường).
- `deny.toml`: GPL-2.0-only (`self_cell`) và LGPL-2.1-or-later (`r-efi`) chỉ xuất hiện như một lựa
  chọn trong biểu thức `OR` mà các crate đó cũng cấp theo MIT/Apache-2.0, nên **không** được cho
  phép; `cargo deny check` vẫn qua ở cả hai workspace. MPL-2.0, NCSA, BSL-1.0 chỉ có ở `desktop/`.
- `find` render vào bản sao ref map rồi gộp lại (`RefMap::absorb`), để ref của lần xem đầy đủ
  cuối vẫn dùng được khi `find` có tuỳ chọn khác.
- Lần chờ tải trước snapshot bị giới hạn bằng `settleMs` (500 ms), không phải 10 s; chỉ nhánh
  thử lại sau khi tài liệu bị huỷ mới chờ tải đầy đủ.
- Batch đánh dấu `_noDelta` cho mọi bước trừ bước cuối có delta (`DELTA_ACTIONS`), không phải
  bước cuối theo vị trí, để một batch kết thúc bằng `screenshot` vẫn có delta.
- Markdown: ô bảng không đóng (`<td>a<td>b`) được đóng theo hàng và bảng; một mục danh sách rỗng
  không nuốt tiêu đề phía sau; thuộc tính được đọc từng cái, nên `href` giả trong giá trị của
  thuộc tính khác không được lấy.
- Sau khi tiêm lại content script, action đổi trang **không** được gửi lại như kế hoạch viết: tài
  liệu mới chưa qua kiểm tra site (blocked sites, PAGE_CHANGED) của lời gọi này.

### 3.3 Rủi ro còn lại

- Chỉ khi tài liệu cũ bị huỷ giữa lúc chụp, snapshot mới chờ `awaitLoad` tới `LOAD_TIMEOUT_MS`
  (10 s); khi đó delta chậm hơn trước nhưng đúng trang mới. Nếu trang chưa tải xong sau 500 ms mà
  tài liệu cũ vẫn còn, delta có thể vẫn là của trang cũ, như trước đây.
- Bỏ phần tử ẩn khỏi markdown cần `getComputedStyle` cho mọi phần tử; trang rất lớn tốn thêm vài
  chục ms mỗi lần đọc. `opacity:0`, `font-size:0` và chữ đẩy ra ngoài màn hình vẫn lọt qua.
- `read <url>` qua HTTP resolve link theo URL trang, bỏ qua `<base href>`.
- Delta sau action có hạn 3 s (`DELTA_BUDGET`); quá hạn thì câu trả lời không kèm delta, để
  client không báo lỗi timeout cho một action đã chạy.
- `waitActionable` làm phần tử thực sự bị ẩn/che thất bại sau 2 s thay vì ngay lập tức.

---

## 4. Hoãn lại

| Mục | Lý do |
| --- | --- |
| Cache `describe`/`offer` theo manifest serial (B3) | Ước tính chỉ ~1–3 ms mỗi `act` |
| Bỏ `childFrames` khi trang không có iframe (B4) | Cần quyền `webNavigation` (cảnh báo cài đặt mới) hoặc một lượt duyệt top frame thứ hai |
| Cache blocked sites / vault trong background | Danh sách blocked sites được đọc mỗi lần kiểm tra có chủ đích (chú thích trong `site-guard.ts`); chưa đo chi phí, phải đo trước |
| Cache `read_agent_config` (B5) | Chỉ vài micro giây |
| Neo cho ref `nth` và ref trong shadow khi ref cũ | Cần thiết kế khoá ổn định; đoán sai sẽ bấm nhầm hàng |
| Phân trang cây (`offset` cho tree) | `find` và `viewport` giải quyết phần lớn; phân trang cây cần ổn định ranh giới dòng với delta |
| CI e2e trên Chromium thật | Cần runner có trình duyệt và fixture; puppeteer đã có trong `bench/tokens` |
| `clippy::unwrap_used` | Nhiều chỗ cần sửa; tách đợt riêng |
| Crate trùng, `ring` + `aws-lc-rs` | Phụ thuộc gián tiếp (reqwest/rustls); cần chọn một backend TLS |
| Chạy lại bench `claude -p` trực tiếp | Tốn lượt chạy trả phí |
| Chú thích WebMCP (`consequentialHint`, `untrustedContentHint`, `readOnlyHint`) và dạng `getTools` của Chrome 160 | Spec còn đang thay đổi |
| `structuredContent` / `outputSchema` | Chỉ có lợi cho client lập trình, không bớt token |
| MCP Tasks | Client hỗ trợ còn mỏng |
| Runner agy/omp | `.omp/plans/NEXT_STEPS_PLAN.md` Phần 1; `crates/vexdom-core/src/agents.rs:8` vẫn là `["claude"]` |
| Thu nhỏ screenshot (cạnh dài ≤1.024 px) | Phải ánh xạ lại toạ độ cho click theo điểm |

---

## 5. Kỹ thuật của đối thủ

Mọi nhận định dưới đây lấy từ **tóm tắt kết quả tìm kiếm** của các URL đã dẫn, chưa đọc toàn văn;
số liệu benchmark do bên thứ ba hoặc chính nhà cung cấp công bố.

| Công cụ | Kỹ thuật | Nguồn |
| --- | --- | --- |
| Playwright MCP | Bỏ snapshot inline, ghi `.yml` rồi trả đường dẫn; `browser_find` tìm text/regex trong snapshot kèm 3 dòng ngữ cảnh; `browser_snapshot` có `target`, `depth`, `boxes`, `filename` | [v0.0.70](https://osfeed.dev/p/microsoft/playwright-mcp/v/v0.0.70), [PR #41605](https://github.com/microsoft/playwright/pull/41605), [snapshots](https://playwright.dev/mcp/snapshots) |
| Playwright MCP | Chi phí thay đổi ~370× theo lượng trang agent đọc, chỉ ~5% giữa MCP và CLI | [uiverify](https://uiverify.ai/blog/playwright-cli-vs-mcp) |
| chrome-devtools-mcp | `--slim` 3 tool; `take_snapshot` có `filePath`, `verbose`; `--autoConnect` dùng lại Chrome đã đăng nhập | [v0.18.0](https://github.com/ChromeDevTools/chrome-devtools-mcp/releases/tag/chrome-devtools-mcp-v0.18.0), [tool-reference](https://github.com/ChromeDevTools/chrome-devtools-mcp/blob/HEAD/docs/tool-reference.md) |
| agent-browser 0.45 | Ref xuyên shadow root mở và đóng; mã lỗi tiền điều kiện ổn định (`not_editable`, `read_only`, …); `snapshot --delta` có baseline đánh số | [v0.45.0](https://github.com/vercel-labs/agent-browser/releases/tag/v0.45.0), [v0.44.0](https://github.com/vercel-labs/agent-browser/releases/tag/v0.44.0), [PR #1811](https://github.com/vercel-labs/agent-browser/pull/1811) |
| browser-use 1.0 | Một REPL Python trên CDP thay 24 action; tự công bố nhanh 2–3×, rẻ 2,7–5,2× | [post](https://browser-use.com/posts/browser-use-1-0) |
| Stagehand v4 | Lõi trong service worker của extension; cache `act`/`observe` theo chỉ dẫn + nội dung trang; `selfHeal` mặc định | [blog](https://www.browserbase.com/blog/stagehand-v4), [caching](https://docs.stagehand.dev/v3/best-practices/caching) |
| Claude in Chrome / browser toolset | `read_page(filter, depth, ref)`, `find` ngôn ngữ tự nhiên (≤20 kết quả), `get_page_text`; báo trạng thái trình duyệt sau mỗi lời gọi | [docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/browser-use-tool) |
| BrowserMCP | Một tool nhận lệnh theo dòng, gộp nhiều bước, dừng ở lỗi đầu | [browser-tool](https://browsermcp.io/docs/browser-tool) |
| Nanobrowser | `viewportExpansion` (-1 tất cả, 0 chỉ viewport, N px); đánh số lại phần tử khi DOM đổi giữa batch | [deepwiki](https://deepwiki.com/sriakula1/nanobrowser/4.3-dom-service-and-element-discovery) |
| WebFovea (nghiên cứu) | Sửa ánh xạ toạ độ screenshot↔viewport, đọc lại giá trị sau khi điền: 8/24 → 20/24 | [arXiv](https://arxiv.org/abs/2610.03036) |
| WebMCP | Draft của Community Group; Chrome 152 bỏ `navigator.modelContext`, Chrome 160 đổi dạng `getTools()` | [updates](https://developer.chrome.com/docs/ai/webmcp/updates) |
| MCP 2026-07-28 | Lõi không trạng thái, `input_required`, `tools/list` cache được, `structuredContent`; extension Tasks | [blog](https://blog.modelcontextprotocol.io/posts/2026-07-28/), [tasks](https://tasks.extensions.modelcontextprotocol.io/specification/2026-07-28/tasks) |

Điểm khác biệt vexdom giữ được: delta ngay trong câu trả lời (Playwright MCP đã bỏ snapshot inline),
catalog 3 tool, và giờ thêm `find`/`viewport` tương đương `browser_find` và `viewportExpansion`.
