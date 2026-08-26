# Caddedit

> 🌐 [English](README.md) | 繁體中文

**拆分、檢視、切換 Caddy 站點區塊——不再痛苦。**

單一靜態執行檔。不用 Python。不用常駐服務。你的 Caddyfile 永遠是唯一真相。

[![Release](https://img.shields.io/github/v/release/ChiuHuang/Caddedit?style=flat-square&color=2563eb)](https://github.com/ChiuHuang/Caddedit/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/ChiuHuang/Caddedit/ci.yml?branch=main&style=flat-square&label=CI)](https://github.com/ChiuHuang/Caddedit/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-dea584?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-3fb950?style=flat-square)](LICENSE)

```bash
curl -sSL https://raw.githubusercontent.com/ChiuHuang/Caddedit/main/install.sh | sudo bash
```

---

```console
$ caddedit ls
      DOMAINS                                   TYPE    UPSTREAM                        TLS
● on   app.example.com                           proxy   localhost:3000                  -
● on   grafana.example.com, metrics.example.com  proxy   localhost:3333, localhost:3334  internal
○ off  legacy.example.net:8443                   static  -                               acme email (me@example.com)
● on   www.example.net                           static  -                               dns challenge (cloudflare)
● on   edge.example.io                           raw     h2c://backend:9000              -
```

## 為什麼

單體 `Caddyfile` 會變成垃圾場；網頁面板又把設定變成跟真實語法對著幹的表單欄位。
**Caddedit 兩邊都不選：**

- 每個站點一個檔案，放在 `vhosts/enabled/` 與 `vhosts/disabled/`，
  **位元組級**搬移——tab、註解、heredoc 原封不動
- 看不懂的語法誠實標為 `raw`，絕不亂改
- 所有變更上線前都先跑 `caddy validate`，驗證或 reload 失敗自動回滾
- 單一靜態 musl 執行檔——`scp` 到任何伺服器就能跑

## 指令

| | | |
| --- | --- | --- |
| `caddedit init` | 拆分單體 Caddyfile | `--force` 重新拆分 |
| `caddedit ls --json` | 所有路由一覽 | 可腳本化輸出 |
| `caddedit show [domain]` | 印出單一站點區塊 | 省略 → 互動式選擇器 |
| `caddedit new app.com` | 建立新路由 | 精靈或旗標 |
| `caddedit on / off <domain>...` | 停用與恢復路由 | 先驗證 + reload |
| `caddedit rm [domain]` | 軟刪除 → backups/ | 永不硬刪 |
| `caddedit edit [domain]` | `$EDITOR`，離開時驗證 | |
| `caddedit check` | 驗證全部 | 適合 cron 的結束碼 |
| `caddedit reload` | 重載 caddy | 支援自訂指令 |
| `caddedit serve` | 選配 MDUI 儀表板 | 內建，無 CDN |

直接執行 **`caddedit`** 是互動式 TUI 瀏覽器：

| 按鍵 | 動作 | 按鍵 | 動作 |
| --- | --- | --- | --- |
| `j/k` | 移動 | `e` | 編輯區塊 |
| `space` | 切換 | `d` + `y` | 刪除 |
| `r` | 重載 caddy | `q` | 離開 |

## 運作方式

```
/etc/caddy/Caddyfile          全域選項 + snippets + import 行
/etc/caddy/vhosts/enabled/    上線中的站點區塊        — 每個路由一個檔案
/etc/caddy/vhosts/disabled/   停用的路由              — 切換只是搬檔案
/etc/caddy/backups/           時間戳備份 + 軟刪除的路由
```

## Web 儀表板

```bash
CADDEDIT_PASSWORD=secret caddedit serve --host 127.0.0.1 --port 29048
```

Material Design 3 介面直接編譯進 binary——路由清單含開關、可編輯的 Parsed
表單與 Raw 模式分頁、帶 TLS 預設值的路由建立、外掛系統、自我更新（含進度條）、
深淺色主題。完全離線可用。

<details>
<summary><strong>環境變數</strong></summary>

| 變數 | 預設 | 用途 |
| --- | --- | --- |
| `CADDYFILE_PATH` | `/etc/caddy/Caddyfile` | 主設定檔路徑 |
| `VHOSTS_DIR` | `<config 上層>/vhosts` | enabled/disabled 根目錄 |
| `CADDY_BACKUP_DIR` | `<config 上層>/backups` | 備份 |
| `CADDY_BIN` | `caddy` | validate/reload 用的執行檔 |
| `CADDEDIT_RELOAD_COMMAND` | `caddy reload --config <path>` | 自訂 reload |
| `CADDEDIT_PASSWORD` | *(未設 = 不上鎖)* | 儀表板密碼 |

</details>

## 從舊版 Python webui 遷移？

`vhosts/` 目錄結構完全相同——裝上新 binary、跑 `caddedit ls`
確認看得到你的路由，然後停掉舊服務即可。完整步驟：
[`cli/README.md`](cli/README.md#migrating-from-the-legacy-python-webui)。
舊 FastAPI 程式碼已移除，需要時可從 git 歷史找回。
