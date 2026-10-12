# Design Decisions(最終定案)

> 2026-10-13 全域討論完成。本文件為唯一設計事實來源。
> 討論過程:`design-review.md`(歸檔)。實作完成後本文件隨 repo 提交。

## 0. 哲學(最高優先,使用者定)

1. 工具描述只描述自身功能;**跨工具 workflow 提示一律不要**
2. 工具自身的參數能力 → **動態 hint 放回傳 JSON 內,對應參數缺省時才出現**(英文)
3. 人眼看到的表格由 **agent 自行渲染**;工具輸出 JSON 是資料原形
4. **markdown 只在「文件即產品」時出現**(課程地圖、作業詳情)
5. 資料截斷 → 乾淨截斷 + 提示更完整的取得方式(download_file / get_discussion_posts)
6. 所有 MCP 輸出、提示、錯誤訊息:**英文**

## 1. 輸出政策

- 所有工具:`CallToolResult::structured(json)` —— `text` = compact JSON 字串、
  `structuredContent` = 同一 JSON(codemode 完整可用;pi 20KB 中段截斷只影響 text)
- **例外(文件工具)**:`get_course_overview` / `get_assignment` 的 `text` =
  markdown 文件(模型直接讀);`structuredContent` = 機讀 JSON(ids/fileurls/計數)
- **null 欄位一律省略**(輸出前遞迴剝除;3.7 缺欄位是常態)
- 文件工具 markdown 超長 → head 截斷 + 標記(總長);一般工具 JSON 永不截斷
- HTML 欄位(說明/訊息/通知內文)一律轉 markdown(`htmd`),淘汰 strip_html

## 2. 最終工具清單(22 個)

```
whoami()                                            自己的身分(userid/username/fullname/site/release)
list_courses(search?)                               選課清單;search=名稱過濾(客户端)
get_course_overview(course_id)                      課程全圖 markdown(5 WS 併發合成)
get_forum_discussions(forum_id)                     討論串列表(含 preview 200 字)
get_discussion_posts(discussion_id)                 整串貼文(message→markdown、attachments)
create_forum_discussion(forum_id, subject, message) 新增討論
reply_to_discussion(post_id, subject, message)      回覆(回標題=回第一篇,同一操作)
get_assignment(assignment_id)                       作業全包 markdown(狀態/時間/嘗試/
                                                    說明全文/提交內容/檔案清單+URL)
submit_assignment(assignment_id, text?,             唯一寫入;text 與檔案可同時;
                   item_id?, finalize?)             finalize=送出評分(不預設,資料驅動)
fetch_file(url)                                     anydoc → markdown;上限 100k 字元
download_file(url, save_to)                         bytes → 磁碟;save_to 必填;無 inline
upload_file(path, item_id?)                         磁碟 → 草稿區;只吃 path
get_grades(course_id?)                              缺省=跨課程摘要(overview API);
                                                    帶=單課全細節(grade items)
get_schedule(time_from?, time_to?, course_ids?)     統一時間軸;預設 now~+30d;
                                                    巢狀行事曆參數(3.7~5.0 通用)
get_notifications(read?, limit=20)                  通知(legacy fallback 內建)
get_conversations(limit=20)                         對話(私聊顯示名取自 members)
send_message(user_id, text)
get_quiz_attempts(quiz_id)                          嘗試記錄(唯讀;做測驗鏈 by design 不實作)
get_user(field, value)
search_users(search?, limit?)                       fallback 鏈內建;權限不足回乾淨錯誤
call_ws_function(wsfunction, params?)               逃生口:原樣 JSON 透傳
list_functions(query?)                              能力清單探索(query 必填,子字串過濾)
```

**移除**(13):site_info、get_assignments、get_course_contents、search_courses、
get_quizzes、get_forums、get_calendar_events、get_upcoming_deadlines、
get_assignment_status、submit_assignment_text、submit_assignment_files、
submit_assignment_for_grading、upload_draft_file
**測驗域整域不實作**(by design):意義不大、工具膨脹、start_attempt 誤觸發即計時的實害;
非學術倫理考量。長尾走 call_ws_function。

## 3. 行為決策(依域)

### 課程/地圖
- `list_courses(search?)`:自己的課;客户端名稱過濾
- `get_course_overview`:併發 contents+assign+forum+quiz+grades 五來源合成一份
  markdown;作業/測驗/論壇/成績彙總表 + 章節逐項(URL 內嵌);**全部顯示不降級**;
  成績區只列**已評分**項目,失敗標 `grades: unavailable (errorcode)`
- `whoami`:讀內部快取的 site_info(移除的工具,內部機制保留)

### 作業
- `get_assignment`:markdown;檔案一律清單+URL;onlinetext 內容與評語例外(純文字內嵌);
  含「需送出評分」旗標與嘗試次數;previousattempts 只給次數
- `submit_assignment.finalize`:不預設;agent 從 get_assignment 旗標判斷
- 回應保持純結果;送出後讀成績 → 再呼叫 get_assignment

### 論壇
- 列表含 `preview`(開題內文截 200 字;標記 "use get_discussion_posts to read the full thread")
- `get_discussion_posts` 無分頁(3.7 不支援,實證);整串一次回
- subject 一律 entity 解碼;locked 欄位名修正;attachments 提取(3.7 實證存在)

### 檔案
- `fetch_file`:anydoc 轉 markdown;>100k 字元 head 截斷 + 標記
- `download_file`:`save_to` 必填;只回 metadata
- `upload_file`:只吃 `path`(content_base64 移除);多檔案=同 item_id 重複呼叫

### 時間軸
- 預設 now~+30d;缺省 hints 兩條;course_ids 缺省=all(內部一律送
  `events[courseids]`,因不帶就拿不到模組事件 — 3.7~5.0 通用實證)

### 訊息/使用者(NTNU 隱私模型實證 2026-10-13)
- `get_conversations` 輸出加 `members: [{id, fullname}]` —— 私聊顯示名 +
  學生 id 的唯一發現來源(名冊被站點鎖、search 被拒)
- 隱私模型實證:對話 members 看得到同學存在;get_user 查學生一律
  "no user found"(站點隱私鎖);email 僅對你的授課老師開放;username 隱藏
- `get_user` 錯誤訊息如實回報查無此人,**不掛原因猜測**
- `send_message` 回應純結果;草擬+確認由 agent 與使用者把關(instructions 已載)

### 逃生口
- `list_functions(query?)`:query 必填(避免 349 條全列)
- 白名單外 → 模糊建議(子字串+編輯距離前 3,"Did you mean: ...?");無近似 → 計數+樣本

## 4. 版本相容(實證 2026-10-13)

| 差異 | 處理 |
|---|---|
| 行事曆參數巢狀 `events[courseids]`+`options[]`(3.7~5.0 相同);不帶 courseids 無模組事件 | 一律帶全部選課 |
| `mod_forum_get_discussions` 3.7 缺 → `get_forum_discussions`(拒 sortby、包 {discussions}) | pick_function |
| `mod_forum_get_discussion_posts` 3.7 拒 page/perpage | invalidparameter 自動降級 |
| `core_message_get_notifications` 3.7 缺 → `core_message_get_messages` | pick_function |
| `core_user_get_users` 3.7 缺 → `core_message_message_search_users` | pick_function |
| `gradereport_overview_get_course_grades` 舊版可用 | get_grades 缺省模式 |
| `core_calendar_get_calendar_events` timefrom 4.1+ 才有 | 一律送全選課 courseids,不依賴 timefrom |

## 5. 依賴

| crate | 用途 |
|---|---|
| `anydoc` 0.2+ | fetch_file 文件轉 markdown(PDF/DOCX/PPTX/XLSX/ODT/RTF/EPUB/CSV;不支援 HTML) |
| `htmd` 0.5+ | HTML → GFM markdown(表格/清單/連結)—— 定案(6.4M 下載、turndown 血統、活躍維護；MSRV 實作時驗證) |
| `rpassword` | token 子命令的隱藏式密碼輸入(不進 shell 歷史) |
| `open` | token --open 開瀏覽器(跨平台) |

## 5.5 CLI 子命令：`token`(定案)

```text
moodle-mcp token --url <site> [選項]
  --username U --password P   帳密模式；password 缺 → 隱藏式 prompt(亦可 stdin)
  --service S                 預設 moodle_mobile_app
  --open                      開瀏覽器到 {site}/user/managetoken.php(SSO 適用，人工複製)
  --output env|json           預設 env(印 export 行)
```

- 端點:`POST /login/token.php`(NTNU 實證可用)；錯誤如實轉述
  (invalid login / service disabled / toomanyrequests)
- 瀏覽器模式端點:`/user/managetoken.php`(Security keys 頁，需登入 session)
- 無子命令 → 現行 MCP stdio serve(向後相容)
- 文件明載：token 視同密碼

## 6. ID 對照表(README 用)

| ID | 產出工具 | 消費工具 |
|---|---|---|
| course_id | list_courses / get_course_overview | overview/grades/schedule/submit 等 |
| forum_id | get_course_overview(地圖) | get_forum_discussions / create_forum_discussion |
| discussion_id | get_forum_discussions | get_discussion_posts |
| post_id | get_discussion_posts | reply_to_discussion |
| assignment_id | get_course_overview / get_grades | get_assignment / submit_assignment |
| item_id(草稿區) | upload_file | submit_assignment |

## 7. 實作檢查清單

- [ ] finish():structured 雙通道 + strip_nulls + hints + 文件工具 markdown 截斷
- [ ] whoami / list_courses(search) / get_course_overview(5 WS 併發)
- [ ] get_assignment / submit_assignment(合併舊三者)
- [ ] fetch_file(anydoc)/ download_file(save_to 必填)/ upload_file(path)
- [ ] get_grades 缺省雙模式 / get_schedule(巢狀參數+30d+hints)
- [ ] 論壇四工具(預覽/attachments/locked 修正/entity 解碼)
- [ ] list_functions + Did-you-mean;移除 13 個舊工具
- [ ] `token` 子命令(帳密模式 + --open 瀏覽器模式)
- [ ] 測試更新 + NTNU e2e(地圖→讀 PDF→上傳→submit 鏈→schedule)
- [ ] README 重寫(ID 對照表/工具表/版本相容)
