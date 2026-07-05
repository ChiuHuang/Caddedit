# Caddedit

Caddedit 是一個專為 Caddy 打造的 Web 視覺化 Caddyfile 路由器管理器 (TXG1 Router)。

它可以將原本單一且雜亂的 `Caddyfile` 拆分（Split）為獨立的 `.caddy` 虛擬主機設定檔，並提供直覺的 GUI 介面進行路由新增、停用/啟用、TLS 策略設定及額外指令（Directives）管理，同時支援安全備份與 Cohere AI 自動解析。

---

## 架構說明

* **前端 GUI 介面**：使用極簡且反應快速的 Single Page HTML / Vanilla JS / CSS 打造。
* **後端伺服器 (FastAPI)**：處理路由的儲存、備份、重載，以及 AI 解析。
* **Caddy 整合配置**：
  * 主設定檔 `/etc/caddy/Caddyfile` 僅保留全域設定與 `import /etc/caddy/vhosts/enabled/*.caddy`。
  * 啟用中的站點存放於 `vhosts/enabled/`，停用中的站點存放於 `vhosts/disabled/`。

> [!NOTE]
> Caddedit 預設會讀取系統上的 Caddy 設定。若有安全性或開源考量，所有的敏感資訊（如密碼、API Key）已完全抽離至 `.env` 檔案中。

---

## 功能特性

* **視覺化路由管理**：可調整網域、狀態（ON/OFF）、反向代理（Reverse Proxy）、TLS 策略（包含 Cloudflare DNS 或自訂 TLS）。
* **極簡 GUI 與 Raw 雙模式**：簡單站點使用 GUI 編輯，複雜站點自動保持 Raw 原始格式，隨時可在 Web 介面一鍵切換。
* **Caddy 自動重載**：儲存路由時，背景自動執行 `caddy reload` 讓設定即時生效。
* **備份機制**：每次修改主設定檔時，系統皆會自動建立時間戳記備份。
* **AI 輔助解析 (可完全停用)**：使用 Cohere v2 將複雜的 Caddy 語法解析為 GUI 欄位。

---

## 環境變數配置 (.env)

| 變數名稱 | 說明 | 預設值/範例 |
|----------|------|-------------|
| `CADDEDIT_PASSWORD` | Web 解鎖密碼 (必填) | `your_secure_password` |
| `CADDYFILE_PATH` | Caddyfile 主設定檔路徑 | `/etc/caddy/Caddyfile` |
| `VHOSTS_DIR` | 虛擬主機（vhosts）儲存目錄 | `/etc/caddy/vhosts` |
| `CADDY_BACKUP_DIR` | Caddy 設定檔備份路徑 | `/etc/caddy/txg1-router-backups` |
| `DISABLE_AI` | 是否完全隱藏並停用 AI 解析功能 | `true` |
| `COHERE_API_KEY` | Cohere API 金鑰 (若啟用 AI 解析則需填寫) | `your_cohere_api_key` |
| `COHERE_MODEL` | 用於解析的 Cohere 模型 | `command-a-03-2025` |
| `CADDY_RELOAD_COMMAND` | 重載 Caddy 的命令 | `caddy reload --config /etc/caddy/Caddyfile` |
| `HARDCODED_RULES_PATH`| 儲存 AI 學習規則的 JSON 路徑 | `/opt/caddedit/hardcoded-rules.json` |
| `PORT` | 服務運行埠 | `29048` |
| `HOST` | 服務綁定 IP | `0.0.0.0` |

> [!IMPORTANT]
> 若將 `DISABLE_AI` 設為 `true`，Web 介面上的 **AI Fallback** 標籤頁將會完全隱藏，且背景 API 亦會拒絕所有 AI 解析請求，確保資料完全保密不外流。

---

## 一鍵安裝與遷移

我們提供了一鍵安裝腳本，支援自動安裝依賴、配置 Systemd 服務、手動輸入密碼，以及**自動備份並拆分現有的 monolithic Caddyfile**。

### 執行安裝

```bash
# 下載安裝腳本並執行 (請以 root 權限執行)
curl -sSL https://chiuhuang.dev/caddedit/install.sh | sudo bash
```

> [!TIP]
> 1. 安裝程式會詢問是否「**自動遷移現有的 Caddyfile 站點**」，回答 `Y` 會自動將您目前的站點依網域拆分寫入 `vhosts/enabled/` 目錄，並在 Caddyfile 中自動寫入 `import` 關聯。
> 2. 原本的 Caddyfile 會備份為 `/etc/caddy/Caddyfile.bak.original`。

---

## 後端 API 端點

| 方法 | 端點 | 需要密碼驗證 | 說明 |
|------|------|:------------:|------|
| `GET` | `/` | 否 | 顯示登入頁面 (未認證) 或 主控制台 (已認證) |
| `POST` | `/api/auth/login` | 否 | 送出密碼認證，成功時寫入 Session Cookie |
| `POST` | `/api/auth/logout` | 否 | 清除認證 Cookie |
| `GET` | `/api/config` | **是** | 取得全域 Caddy 設定、所有拆分的 vhost 路由及 AI 開關狀態 |
| `PUT` | `/api/config` | **是** | 儲存/覆寫原始 Caddy 主設定檔內容 |
| `POST` | `/api/routes` | **是** | 建立新的站點路由（寫入至 vhosts 啟用資料夾） |
| `PUT` | `/api/routes/{route_id}` | **是** | 更新指定站點路由內容（支援重新檔名轉換與狀態切換） |
| `DELETE`| `/api/routes/{route_id}` | **是** | 刪除指定站點路由檔案 |
| `POST` | `/api/routes/{route_id}/toggle`| **是** | 快速切換啟用 (ON) / 停用 (OFF) 狀態 (自動移動檔案於 enabled/disabled 間) |
| `POST` | `/api/reload` | **是** | 手動發送 Caddy 重載命令 |
| `POST` | `/api/hardcoded-rules` | **是** | 儲存經由 AI 學習而來的解析特徵規則 |
| `DELETE`| `/api/hardcoded-rules` | **是** | 刪除已儲存的 AI 學習特徵規則 |
| `POST` | `/api/ai/parse` | **是** | 調用 Cohere 將 site block 原始碼解構成 GUI 欄位 JSON (若停用則回傳 400) |

---

## 專案結構

```text
/opt/caddedit/
├── manager.py          # FastAPI 主程式邏輯
├── templates/          # HTML 模板資料夾
│   ├── login.html      # 登入介面
│   └── index.html      # 路由管理器主控制台
├── .env                # 環境設定檔 (安全隔離，Git 已忽略)
├── .env.example        # 環境設定範例檔
├── pyproject.toml      # 專案依賴描述 (FastAPI, uvicorn 等)
└── uv.lock             # uv 鎖定檔
```
