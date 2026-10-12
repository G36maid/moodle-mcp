# 工具設計檢討筆記 v4 — 入口→地圖→精準工具(分輪討論中)

> 核心模型(使用者定):
> 1. **入口**:agent 一定先了解有哪些課程 → 一個精簡的 `list_courses`(list/search 本質同一工具,類似 context7 的 resolve-library-id)
> 2. **地圖**:一個課程用**一份 markdown** 表達完整結構 —— 大段落、檔案、作業、討論區、測驗,全部內嵌 ID
> 3. **精準工具**:agent 從地圖上看到什麼,就用對應的精簡工具深入

---

## 〇、討論輪次與狀態

| 輪 | 主題 | 狀態 |
|---|---|---|
| ① | 入口 + 課程地圖(list_courses / get_course_overview) | ✅ 大致定案,剩 D6' converter |
| ② | 作業域(get_assignment / upload_file / submit_assignment) | ✅ 定案(見下) |
| ③ | 檔案處理(fetch_file / download_file / upload_file、大小上限) | ✅ 定案(見下) |
| ④ | 其他域(whoami / get_schedule / user_id 移除) | ✅ 定案(見下) |
| ④五 | call_ws_function 逃生口(list_functions + 模糊建議) | ✅ 定案(見下) |
| ⑤ | 全域輸出政策(null 省略) | ✅ 定案 |

已定案:
- 地圖**全部顯示**,不做分級降級;真需要再加 search 參數(YAGNI)
- 檔案 URL **直接內嵌** markdown
- `list_courses(search?)` **只過濾自己的課**(客户端過濾);目錄搜尋不做
- `fetch_file` / `download_file` / `upload_file` 的細節 → 輪③
- `get_assignments` 的去留 → 輪②(fetch PDF、讀論壇、查成績)

---

## 一、新工具形狀

### 入口層
```
list_courses(search?)
  → 無 search:回全部選課(id, 名稱, 進度, 短摘要)
  → 有 search:名稱過濾(客户端)—— list 與 search 合一
```

### 地圖層(核心新工具)
```
get_course_overview(course_id) -> markdown
```
一份人/LLM 可讀的課程全圖,server 內部併發打 contents+assign+forum+quiz 四個 WS,合成:

```markdown
# 1151資料通訊 (Data Communication) — course_id: 61716

## 作業 (assignments)
| id | 名稱 | 截止 | 附件 |
|---|---|---|---|
| 106261 | HW1 | 2026-10-15 | 2026_Information_Security_HW01.pdf |
| 107826 | Midterm | 2024-10-26 | — |

## 測驗 (quizzes)
| id | 名稱 | 開放 | 關閉 |
|---|---|---|---|

## 討論區 (forums)
- Announcements (公告) — forum_id: 145519
- 作業討論 — forum_id: 145520

## 章節內容
### Week 1: Introduction
- [檔案] Lecture1.pdf — 1.2 MB → fileurl
- [網頁] 課程大綱 — description markdown...
### Week 2: Delay Models
- [檔案] slides_week2.pptx — 3.4 MB → fileurl
- [作業] HW2 — assignment_id: 106681, due 2026-10-22
```

特性:
- ID 全部內嵌 → agent 看圖即知該呼叫什麼(作業→submit 鏈、論壇→discussions、檔案→fetch/download)
- markdown 大綱比巢狀 JSON 密 5-10 倍
- 4 個來源去重:作業/測驗/討論區有彙總表,章節內只標記引用

### 精準層(不變或微調)
```
whoami()                   自己的身分(userid/username/fullname/site/release)
get_assignment(assignment_id)  讀:狀態/時間/嘗試/說明全文/提交內容/檔案清單
submit_assignment(assignment_id, text?, item_id?, finalize?)
fetch_file(url)            讀 PDF/DOCX/... → markdown(anydoc)
download_file(url, save_to) bytes → 磁碟(save_to 必填,base64 移除)
upload_file(path, item_id?) 磁碟 → 草稿區(content_base64 移除)
get_grades(course_id?)                        缺省=跨課程摘要;帶=單課全細節
get_forum_discussions(forum_id) / get_discussion_posts(discussion_id)
create_forum_discussion / reply_to_discussion
get_notifications / get_conversations / send_message
get_quiz_attempts(quiz_id) / get_user / search_users
call_ws_function(wsfunction, params?) / list_functions(query?)
get_calendar_events / get_upcoming_deadlines(跨課程,S1 儀表板用)
```
(site_info 工具移除,新增 whoami;內部仍快取 site_info 供
 current_user_id/pick_function/版本判斷;
 get_assignments/get_quizzes/get_forums/get_course_contents/search_courses 移除)

---

## 二、被取代/合併的工具

| 舊 | 命運 |
|---|---|
| `get_course_contents` | **由 get_course_overview 取代**(巢狀 JSON 退場) |
| `search_courses` | **併入 list_courses(search?)**(目錄搜尋少用;若要保留用 flag) |
| `get_assignments` | **擬移除**:單課程→地圖已含;跨課程→get_upcoming_deadlines |
| `get_quizzes` | **擬移除**:地圖已含測驗表 |
| `get_forums` | **擬移除**:地圖已含論壇清單 |

27 → 約 21 個工具。

---

## 三、輪① 已定案:入口 + 地圖

```
list_courses(search?)  — 自己的課,客户端名稱過濾
get_course_overview(course_id) -> markdown — 全部顯示,URL 內嵌
```

剩餘小項:
- D6':HTML 欄位(公告/作業描述)轉 markdown 的 converter 選型

---

## 四、輪② 定案:作業域(3 個工具)

讀與寫分離,對應人類在 Moodle 的兩個場景:

```
get_assignment(assignment_id) -> markdown   讀:狀態/時間/嘗試次數/說明全文/
                                            提交內容(文字內嵌)/檔案清單+URL
upload_file(path, item_id?)                寫①:進草稿區(可重複呼叫同一 item_id 聚多檔)
submit_assignment(assignment_id, text?, item_id?, finalize?)
                                           寫②:text 與檔案可同時;finalize 才送出評分
```

決策:
- **檔案一律只給清單+URL**,內容由 fetch/download 額外呼叫(onlinetext 文字與
  評語例外:純文字直接內嵌)
- **finalize 不預設**:agent 從 get_assignment 的「需送出評分」欄位自行判斷;
  保留「存草稿但不送出」的緩衝
- **upload 與 submit 分開**(緩衝空間):上傳是慢動作、可漸進；submit 失敗
  不重傳檔案；多檔案 = 同 item_id 重複 upload
- **不做網址參數**:要網址的檔案由 agent 先 download_file 再 upload_file
- `get_assignments` 移除:單課程→地圖；跨課程→deadlines;詳情→get_assignment
- 團體作業(teamsubmission)暫不支援

---

## 五、輪③ 定案:檔案處理

```
fetch_file(url)            anydoc → markdown;上限 100k 字元;超過 head 截斷
                           + 標記(總長 + 提示 download_file)
download_file(url, save_to) save_to 必填;bytes → 磁碟;只回 metadata;
                           無 inline 模式(base64 移除)
upload_file(path, item_id?) 只吃 path;content_base64 移除
```

調查依據(2026-10-13):
- markitdown-mcp 原始碼:無任何上限(全文回傳)
- pi 核心:MCP 結果 >20KB → 中段截斷 + 完整內容存暫存檔(路徑給 agent)
  → server 回 20k–100k 之間的內容,pi 會自動變成 head+tail+暫存檔路徑,
    agent 仍可讀全文,故 server 不需截太狠
- pi-web-access:`MAX_INLINE_FILE_CHARS = 1e5`(100k 字元),head 截斷
  → fetch_file 對齊 100k
- anydoc:使用者確認為最佳 markdown 轉換工具(優於 markitdown/pdftotext);
  效果不好時 agent 會自行 download 後處理
- save_to 必填(使用者考慮過預設 /tmp,決定必填較好)

---

## 五點五、輪④:其他域 —— 定案

| # | 議題 | 狀態 |
|---|---|---|
| ④-1 | site_info 工具移除 → 新增 whoami;functions 清單延到 call_ws_function 輪 | ✅ |
| ④-2 | ~~get_calendar_events~~ + ~~get_upcoming_deadlines~~ → **合併為 get_schedule** | ✅(見下) |
| ④-3 | get_schedule 輸出格式 | ✅ JSON 雙通道(見「輸出格式」) |
| ④-4 | list_courses/get_grades 的 user_id 參數移除(永遠查 token 本人；list_courses 內部自動帶快取 userid，get_grades 不送 userid 欄位) | ✅ |
| ④-5 | get_quiz_attempts 的 user_id 同步移除 | ✅(與 ④-4 一致) |
| ④-6 | 名冊工具(課程參與者列表) | ✅ **不加**:NTNU 實證學生只見自己+教師(站點隱私設定);組員 id 走對話 members;真需求走 call_ws_function 兜底 |

### get_schedule(取代 get_calendar_events + get_upcoming_deadlines)

```
get_schedule(time_from?, time_to?, course_ids?)
內部:core_calendar_get_calendar_events(events[courseids]=全部選課,
                                      options[timestart/timeend])
     → 依 timestart 排序 → 過濾
     ※ course_ids 選填預設 all(對照 Moodle 月曆檢視的課程下拉選單);
       技術上也是必要行為:不帶 courseids 就拿不到模組事件(全版本通用)
```

**時間窗預設(定案)**:
- `time_from` 缺省 = **now**;`time_to` 缺省 = **now + 30 天**(預設未來一個月)
- 無 limit 參數(30 天窗自然有界)
- 輸出為 JSON 雙通道(structuredContent + text)，依 timestart 升冪；
  表格式工具不做 markdown 渲染(見「輸出格式」)
- **缺省參數的回傳內容提示(英文，僅在該參數被省略時出現)**:
  - "Time range defaults to the next 30 days; pass time_from/time_to to query other ranges."
  - "Defaults to all enrolled courses; pass course_ids to filter."
- 原則(使用者定)：工具描述只描述自身；跨工具 workflow 提示不要；
  但「自身參數能力」的動態提示放在回傳內容，缺省時才出現
- 歷史查詢 = agent 顯式帶窗，opt-in

### 輸出格式(定案，修正版)

人眼看到的永遠是 agent 自行渲染的回答，不是工具輸出。
因此不需要為「人類閱讀」做 markdown 表格渲染：

- **一般工具**:`CallToolResult::structured(json)` 一條碼路 ——
  text = compact JSON 字串，structuredContent = 同一 JSON。
  (codemode 吃 structuredContent；直接呼叫時 LLM 讀 JSON 文字)
- **文件工具(get_course_overview / get_assignment)**:
  text = **markdown 文件**(這就是產品本身，模型直接讀地圖推理)；
  structuredContent = 機讀 JSON(ids/fileurls/計數，codemode 免解析)
- **G4 降級為小題**：text 通道截斷不再有壞後果
  (structuredContent 永遠完整；文件工具的 markdown 截斷是純文字)
- hints 放 JSON 內，兩個通道自然都含
- 不再做 per-tool markdown 表格渲染器
```

**版本行為實證(2026-10-13,原始碼 + NTNU 實測)**:
- 3.7/4.0/4.5/5.0 簽名逐字相同:巢狀 events[courseids] + options[]
- 函式主體:`if ($hassystemcap || !empty($courses))` — 一般使用者
  **不帶 courseids 就拿不到模組事件**(NTNU 回 0 筆的真相;20 筆 due 事件
  一直都在,只是要指名課程)
- 帶上全部選課的 courseids → 一個呼叫涵蓋作業 due/測驗 close/手動事件
  → 不需要 action events API、不需要多來源合併
- 我們的 client 原本送平坦參數(在任何版本都會 invalidparameter)— 已知錯誤

---

## 五點六、輪④:成績域 —— 定案(三鏡頭，缺省合併)

```
get_grades(course_id?)
  有 course_id → 單課全細節(gradereport_user_get_grade_items:所有 grade items,
                 含測驗/手動項/Course total/評語)
  無 course_id → 跨課程摘要(gradereport_overview_get_course_grades,
                 NTNU 可用實證)+ hint: "Pass course_id for full grade items"
```

| 鏡頭 | 工具 | 場景 | 內容 |
|---|---|---|---|
| 跨課程摘要 | get_grades() | 「我這學期如何」 | 每門課最終成績 |
| 地圖內摘要 | get_course_overview | 單課整體狀況 | **只列已評分項目**(考完/改完的)，緊湊表 |
| 課程全細節 | get_grades(course_id) | 所有 grade items | 含測驗/手動項/Course total(非作業項只有它涵蓋) |
| 單作業細節 | get_assignment(id) | 單作業成績+評語 | 本就內含(submission_status) |

- submit_assignment 回應保持純結果；送出後讀成績 → 再呼叫 get_assignment
- 地圖成績區塊失敗時標 grades: unavailable (errorcode)

### 測驗域 —— 定案:整域不實作(by design)

事實:token 可存取完整做測驗鏈(start_attempt → get_attempt_data →
save_attempt → process_attempt);mod_quiz 是完整考試引擎(小考~期末)，
內建平台層規則(timeopen/close、timelimit、password、attempts)API 自動執行。

**理由(使用者定，非學術倫理)**:
1. 意義不大 —— 測驗是單次定時行為，不是可重複的工作流
2. 工具膨脹 —— 4 個工具服務一個低頻場景
3. **誤觸發實害** —— start_attempt 即開始計時，agent 探索/誤解意圖時
   意外開卷是實際傷害(作業提交尚有 draft 緩衝，測驗沒有)

- get_quiz_attempts(唯讀嘗試記錄)保留;其餘整鏈不做
- 未來真有需求 → call_ws_function("mod_quiz_start_attempt", ...) 兜底
  (屆時責任清楚在使用者)

### 訊息/使用者域補充 —— NTNU 隱私模型實證(2026-10-13)

實測(對話成員 × get_user 交叉):
- 對話 members 看得到同學「存在與名字」(學號制帳號,如 61126002L 戴如意)
- 但 get_user(field=id) 查**學生一律回 "no user found"**(站點隱私鎖,非真不存在)
- 教職員查得到(fullname/first_access),但 **email 僅對你的授課老師開放**
- username 欄位一律隱藏(3.7 by-id 回應無此欄)

設計連動:
- **get_conversations 輸出加 `members: [{id, fullname}]`** —— 私聊顯示名 +
  學生 id 的唯一發現來源
- get_user 的錯誤訊息不說謊(已分離 NonJsonResponse/InvalidBody);
  查無此人如實回報,不猜原因
- 名冊不加的決策與此一致(站點隱私本來就關)

### 論壇域 —— 定案(top-down 導航 4 工具)

```
get_course_overview(course_id)      層0:地圖含各論壇 forum_id
get_forum_discussions(forum_id)     層1:討論串列表(subject 解碼/作者/時間/
                                    numreplies/pinned/locked/has_attachment/
                                    preview=開題內文截 200 字)
get_discussion_posts(discussion_id) 層2:整串貼文(message→markdown、attachments)
create_forum_discussion(forum_id, subject, message)
reply_to_discussion(post_id, subject, message)   回標題=回第一篇,同一操作
```

- 無分頁參數(爆量靠截斷防線);posts 的 3.7 不支援分頁(實證)
- preview 截斷標記:"…truncated; use get_discussion_posts to read the full thread."
- numunread 不做(導航模型中 agent 會重讀整串)
- 自動修復:locked 欄位名(3.7 是 locked)、subject entity 解碼、attachments 提取

---

## 六、輪④五:call_ws_function(逃生口)—— 定案

定位:LLM 自己寫 Moodle API 呼叫(運輸層 + 安全層)，覆蓋所有未封裝的長尾。
安全邊界不變:白名單 + Moodle capability 全部生效。

定案:
- **B:新增 `list_functions(query?)`** — 對快取清單做子字串過濾
  (query="wiki" → 3 筆)；無 query → 錯誤 + 說明用法(避免 349 條全列)
- **C:白名單外錯誤訊息加模糊建議** — 子字串 + 編輯距離取前 3 個
  closest match("Did you mean: ...?")；無近似才退回計數+樣本
  (同 pi codemode 打錯工具名時的提示 UX；錯誤訊息即文件)
- 逃生口維持原樣 JSON 透傳(裸輸出是它只當逃生口的原因)

---

## 七、輪⑤ 定案:全域輸出政策

- **null 欄位一律省略**(輸出前遞迴剝除;3.7 缺欄位常態)
- ID 語義:參數文件已含出處;README 補對照表(純文件)
- G4(JSON 截斷)已被雙通道設計解掉，非決策點
