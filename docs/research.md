# Moodle MCP Server — 前期調查報告

> 調查日期:2026-10-13,由 3 個研究 subagent 平行完成(moodle-api-research / moodle-mcp-survey / rust-mcp-ecosystem)。

## TL;DR 決策摘要

| 決策項 | 結論 |
|---|---|
| MCP SDK | **`rmcp = "3.5"`**(官方,3.5.1,MSRV 1.88),features: `server`, `macros`, `transport-io` |
| HTTP client | `reqwest 0.13` + `tokio 1.x` |
| Moodle 存取 | `POST webservice/rest/server.php?wstoken=...&wsfunction=...&moodlewsrestformat=json`,form-urlencoded(PHP 陣列 notation),**不是 RESTful** |
| 認證 | Web service token(env `MOODLE_URL` / `MOODLE_TOKEN`);token 來源建議走 mobile service(`moodle_mobile_app`)或 admin 手動建立 |
| 傳輸 | 預設 stdio;(可選)Streamable HTTP |
| 生態定位 | **無任何 Rust 實作的 moodle-mcp**(TS 6、Python 5、PHP 2、Go 1)→ 明確空缺 |
| 回應解析 | serde untagged enum 同時處理成功結構與 `{exception, errorcode, message}`(錯誤時 HTTP 仍回 200) |

---

## 1. Moodle 存取方式

### 1.1 端點與呼叫格式

```
POST https://<site>/webservice/rest/server.php
  ?wstoken=<token>
  &wsfunction=<function_name>
  &moodlewsrestformat=json
  &<function params>          // application/x-www-form-urlencoded
```

- 參數**不支援 JSON body**;陣列用 PHP notation:`users[0][username]=...`
- 錯誤時 HTTP 常仍是 200,body 為:
  ```json
  { "exception": "moodle_exception", "errorcode": "invalidtoken", "message": "..." }
  ```
  常見 errorcode:`invalidtoken`、`accesscontrolexception`、`nopermission`、`servicenotavailable`、`sitemaintenance`
- 核心**沒有** restful/clean-URL 端點(`webservice_restful` 是第三方外掛,不依賴)

### 1.2 Token 取得

| 方式 | 說明 |
|---|---|
| `/login/token.php` | `POST username/password/service=moodle_mobile_app` → `{"token": "...", "privatetoken": "..."}` |
| Admin 手動 | `Site administration > Server > Web services > Manage tokens`(管理員唯一方式;可設 IP 限制、到期日) |
| Security keys 頁 | 使用者個人頁面直接檢視 |

### 1.3 啟用需求(伺服器端設定)

1. `Advanced features` → Enable web services
2. `Manage protocols` → 啟用 REST
3. 建立 External Service 並加入 functions(或使用內建 `moodle_mobile_app`)
4. Token 需 `moodle/webservice:createtoken` capability

### 1.4 權限限制機制(6 層)

Service 白名單 → required capability → 函式內部 capability check → authorised users → token(validuntil/IP restriction)→ 協定 capability(`webservice/rest:use`)。
**最佳實務**:自建 custom service 只開 read 函式;啟動時以 `core_webservice_get_site_info` 回傳的 `functions` 動態判定可用工具。

### 1.5 檔案上傳/下載

- 下載:`webservice/pluginfile.php/<path>?token=<wstoken>`(`core_course_get_contents` 回傳的 URL 已自動改寫)
- 上傳:`POST webservice/upload.php`(multipart,`token`/`filepath`/`itemid`)→ 檔案進 draft area → itemid 填入後續函式(如 `plugindata[files_filemanager]`)
- Service 設定需勾選 can download/upload files

### 1.6 版本差異

- Moodle 5.0(2025-04)移除 `mod_chat_*`、`mod_survey_*`;5.1/5.2 已陸續發布
- 核心 REST 端點格式自 2.x 未變,向後相容性高
- **不要 hardcode 工具清單**,以 `site_info` 的 `functions` 為準

### 1.7 常用 WS Functions 對照表

| 領域 | WS function | 備註 |
|---|---|---|
| 連線偵測 | `core_webservice_get_site_info` | 無 capability 檢查;回傳 userid/username/functions/release |
| 我的課程 | `core_enrol_get_users_courses` | ⚠️ 沒有 `core_enrol_get_enrolled_courses` 這個函式 |
| 課程時間軸 | `core_course_get_enrolled_courses_by_timeline_classification` | inprogress/past/future |
| 課程內容 | `core_course_get_contents` | sections → modules → contents(含檔案 URL) |
| 作業 | `mod_assign_get_assignments` / `get_submissions` / `get_grades` | 2.4+ |
| 作業提交 | `mod_assign_save_submission` / `submit_for_grading` | 寫入 |
| 教師批改 | `mod_assign_save_grade` | 寫入 |
| 成績 | `gradereport_user_get_grade_items` | 3.2+ |
| 行事曆 | `core_calendar_get_calendar_events` | 需 calendar capabilities |
| 訊息 | `core_message_send_instant_messages` | 寫入,需 `moodle/site:sendmessage` |
| 討論區 | `mod_forum_get_forums_by_courses` / `get_discussions` / `get_discussion_posts` | |
| 使用者 | `core_user_get_users` / `get_users_by_field` | 搜尋需 viewdetails 權限 |
| 檔案上傳 | `webservice/upload.php` 或 `core_files_upload` | base64/multipart |

### 1.8 安全性注意

- 核心**無 rate limiting** → 建議反向代理層處理;用專用服務帳號
- 全程 HTTPS(`login/token.php` 是明文帳密換 token)
- 每個 WS 呼叫模擬 session、有 DB session lock,高併發注意
- 建議 MCP server 內建簡單的請求序列化/節流

---

## 2. 現有 moodle-mcp 方案(2026-10)

### 2.1 主要專案

| Repo | 語言 | Stars | 特色 |
|---|---|---|---|
| [peancor/moodle-mcp-server](https://github.com/peancor/moodle-mcp-server) | TS | 44 | 教師視角;stdio;已知 quiz 功能與文件不符、courseId 硬編碼 |
| [loyaniu/moodle-mcp](https://github.com/loyaniu/moodle-mcp) | Python | 40 | 學生視角 ~30 tools;呼叫不存在的 WS function、MCP 2.x 相容性壞掉 |
| [Snaw80/moodle-mcp](https://github.com/Snaw80/moodle-mcp) | Python | 4 | 打磨最好的 UX;完整 token 取得 helper(SSO/Playwright/moodlemobile://);download_file 自動轉 token-aware URL |
| [onbirdev/moodle-webservice_mcp](https://github.com/onbirdev/moodle-webservice_mcp) | PHP plugin | 17 | 動態把所有 enabled functions 轉成 tools;open issue:nullable schema 被 Claude API 400、void outputSchema 無效 |
| [ali205412/moodle-mcp](https://github.com/ali205412/moodle-mcp) | PHP plugin | 1 | Streamable HTTP + OAuth + capability 分析,活躍 |
| [oktayevin/moodle-student-mcp](https://github.com/oktayevin/moodle-student-mcp) | JS | 0 | 唯一非 stdio(SSE+HTTP)、multi-tenant(X-Api-Key header);有 submit_assignment_text/file |
| [MarcosNahuel/moodle-mcp](https://github.com/MarcosNahuel/moodle-mcp) | TS | 1 | 40 個高階 facade + `ws_raw` 逃生口;寫入以 idnumber upsert 保冪等;需自製 PHP companion plugin |

### 2.2 共同模式

- 15/15 全部用 web service token;主流傳輸是 stdio
- 唯讀查詢為絕對主流:site_info / my_courses / course_contents / assignments / grades / calendar / forum / quiz
- 寫入工具罕見:save_grade、save_submission、forum 發文
- 兩大陣營:教師視角(批改)vs 學生視角(查詢/提交);高星專案單一視角做深

### 2.3 我們要避開的坑

1. 呼叫**不存在的 WS function**(peancor #4、loyaniu #8/#9)→ 所有函式對照官方清單驗證
2. courseId 硬編碼 → 工具參數一律動態
3. 動態 schema 產出不合規 JSON Schema → Rust 靜態型別 + schemars 天然避開
4. SSO 環境拿 token 困難 → 提供 token 取得說明/helper
5. SDK 升級破壞安裝 → pin 版本

---

## 3. Rust MCP 生態系

### 3.1 選型:`rmcp`(官方)

- crates.io `rmcp` 3.5.1(2026-10-05),~3,490 萬下載,dependents 2,197
- MCP 2026-07-28 規範;**MSRV 1.88**;stdio + Streamable HTTP 穩定
- 其他 crate 全數落後 1–2 個數量級(rust-mcp-sdk 次選、mcp-core 已停更、mcp-server 已被取代)

### 3.2 架構寫法

```toml
[dependencies]
rmcp = { version = "3.5", features = ["server", "macros", "transport-io"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "io-std"] }
serde = { version = "1", features = ["derive"] }
schemars = "1"
reqwest = { version = "0.13", features = ["json"] }
```

- `#[tool_router]` + `#[tool(description=...)]` + `#[tool_handler]`;參數 `Parameters<T>` + `Deserialize + JsonSchema`;doc comment → schema description;`Option<T>` → 非 required
- `service.serve(stdio()).await?` → `RunningService` → `.waiting().await?`

### 3.3 錯誤處理雙軌(關鍵)

- **Tool-level**: `Ok(CallToolResult::error(vec![Content::text(...)]))` → `isError: true`,模型可讀取並自修正 → Moodle API 404/權限不足等場景用這個
- **Protocol**: `Err(ErrorData)` → JSON-RPC 錯誤,模型看不到訊息 → 只留給路由/基礎設施錯誤

### 3.4 陷阱清單

1. stdio 模式 stdout 是 JSON-RPC 專用 → tracing_subscriber 必须 `.with_writer(std::io::stderr)`
2. MSRV 1.88,CI toolchain 需 ≥ 1.88
3. 網路教學多為 0.x 舊 API,勿照抄;以 docs.rs latest 為準
4. claude-code/codex 舊版對 HTTP transport 有相容性問題(stdio 影響小)
5. HTTP stateless 模式不要依賴跨請求 in-memory 狀態,共享狀態放 `Arc`

### 3.5 測試模式

- in-process:`tokio::io::duplex` + `().serve(client_t)` 官方測試模式
- Moodle API:wiremock/httpmock mock reqwest
- 煙霧測試:`TokioChildProcess` spawn 真正 binary

---

## 4. 建議架構

```
┌─────────────────────────────────────────────┐
│ moodle-mcp (Rust binary)                    │
│                                             │
│  rmcp stdio transport                       │
│       │                                     │
│  #[tool] handlers  ── Parameters<T>         │
│       │                                     │
│  MoodleClient (reqwest)                     │
│   - form-urlencoded + PHP array notation    │
│   - untagged enum: Ok(T) | MoodleException  │
│   - pluginfile.php 下載 / upload.php 上傳    │
│       │                                     │
│  Moodle REST API (wstoken)                  │
└─────────────────────────────────────────────┘
```

**MVP 工具集**(對應最高頻共通操作):

| Tool | WS function |
|---|---|
| `site_info` | `core_webservice_get_site_info` |
| `list_courses` | `core_enrol_get_users_courses` |
| `get_course_contents` | `core_course_get_contents` |
| `get_assignments` | `mod_assign_get_assignments` |
| `get_assignment_status` | `mod_assign_get_submission_status` |
| `get_grades` | `gradereport_user_get_grade_items` |
| `get_calendar_events` | `core_calendar_get_calendar_events` |
| `get_forum_discussions` | `mod_forum_get_discussions` |
| `search_users` | `core_user_get_users` |
| `call_ws_function`(逃生口) | 任意(驗證是否在 site_info.functions 內) |

**來源**:
- https://docs.moodle.org/500/en/Using_web_services
- https://docs.moodle.org/dev/Creating_a_web_service_client
- https://docs.moodle.org/dev/Web_service_API_functions
- https://moodledev.io/docs/apis/subsystems/external/files
- https://moodledev.io/general/releases/5.0
- https://crates.io/crates/rmcp · https://docs.rs/rmcp
- https://github.com/modelcontextprotocol/rust-sdk

---

## 5. 真實站點驗證(2026-10-13,NTNU moodle3)

以帳號實測 `https://moodle3.ntnu.edu.tw`(Moodle **3.7+ Build 20190614**,349 個行動服務函式),全部 27 個工具打通。過程發現並修正的版本差異:

| 發現 | 影響 | 修正 |
|---|---|---|
| `site_info.functions` 是**物件陣列** `[{name, version}]`,非字串 | `pick_function` 全部匹配失敗 | 解析 `name` 欄位 |
| `mod_forum_get_discussions` 不在 3.7 行動服務 | 論壇工具失效 | fallback `mod_forum_get_forum_discussions`(不可送 `sortby`、回應包在 `{discussions}`) |
| `mod_forum_get_discussion_posts` 在 3.7 **不接受** `page`/`perpage` | invalidparameter | 偵測 `invalidparameter` 自動降級重試 |
| `core_message_get_messages` 列在清單中但已 deprecated | 直接呼叫會 `dml_missing_record_exception`?**否** — 實測可用 | `get_notifications` fallback 用它(type=notifications) |
| `core_user_get_users` 不在 3.7 行動服務 | 學生無法全域搜尋 | fallback `core_message_message_search_users`(仍有 capability 限制 → 清楚的錯誤訊息) |
| `core_calendar_get_calendar_events` 的 `timefrom` 是 4.1+ 參數 | 3.7 靜默忽略 | 客戶端時間過濾(兩版皆通) |
| 3.7 conversation 結構:`type`(非 subtype)、`messages[]`(非 lastmessage)、無 `timemodified` | 對話列表欄位全 null | 雙形狀相容解析 |

實測結果:18 門課、81 個作業、17 個 45 天內 deadline、成績 7 items、51 個論壇、通知與對話正常。

**教訓**:「文件說的函式清單」與「站點實際啟用+參數簽名」是三回事;`pick_function` 動態選擇 + `invalidparameter` 降級重試是必要的防禦設計。
