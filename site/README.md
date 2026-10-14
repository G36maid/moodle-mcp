# moodle.ntnu.cc — 官網

[moodle-mcp](https://github.com/G36maid/moodle-mcp) 的宣傳／說明站台，部署於
**moodle.ntnu.cc**（網域由 [domain.ntnu.cc](https://domain.ntnu.cc) 申請）。

設計語言取自 [zed.dev](https://zed.dev)（紙感網格、髮線、十字標記、kbd 提示、
襯線標題 + 等寬內文），配色使用 Moodle 官方品牌色
（`#F98012` Orange / `#194866` Blue / `#282828` Black，見 Moodle Global Brand
Guidelines）與 NTNU 師大紅 `#C8102E`、師大藍 `#003865`。

## 開發

```bash
npm install
npm run dev        # http://localhost:4321
```

## 建置與部署

```bash
npm run build      # 產出靜態檔至 dist/
```

`dist/` 為純靜態網站，丟上任何 static host 即可，例如：

- **GitHub Pages**：`.github/workflows` 加一個 upload-pages-artifact 步驟
- **Caddy / nginx**：`root * /var/www/moodle.ntnu.cc; file_server`
- **Cloudflare Pages / Netlify / Vercel**：build command `npm run build`，
  output 目錄 `dist`

## 結構

```
src/
  layouts/Base.astro    # <head>、字體、主題初始化（無閃爍）、Header/Footer、快捷鍵
  components/Header.astro
  components/Footer.astro
  pages/index.astro     # 首頁（映射 zed.dev 首頁的段落結構）
  pages/tools.astro     # 22 工具參考（可篩選）
  pages/404.astro
  styles/global.css     # 設計代幣：light/dark 以 [data-theme] 切換
public/
  assets/moodle-mcp-mark.svg   # 新版標誌（取自 banner 左側 mark）
  assets/moodle-mcp-banner.*   # OG image 與素材
```

## 快捷鍵

| 鍵 | 動作 |
|---|---|
| `D` | GitHub Releases（下載） |
| `G` | GitHub Repo |
| `T` | 切換深淺色主題 |
| `Ctrl/⌘ + K` | 工具參考頁 |

主題偏好存於 localStorage，未設定時跟隨 `prefers-color-scheme`。

## 商標聲明

本站為學生社群專案，與 Moodle Pty Ltd. 及國立臺灣師範大學皆無隸屬關係。
Moodle™ 為 Moodle Pty Ltd. 之註冊商標。
