# Garmin MCP Rust 與 Python 版的差異與優勢

本文比較我們的 `garmin_mcp_rust` 與原始 Python 專案 [`Taxuspt/garmin_mcp`](https://github.com/Taxuspt/garmin_mcp) 的設計取向、優勢與目前差距。重點不是單純說哪個「比較強」，而是清楚說明：**兩者的優化方向不同**。Python 版偏向「功能覆蓋完整、易上手、MFA/預先認證成熟」；我們的 Rust 版則偏向「部署簡單、執行更穩、併發與 session 安全更明確、研究輸出更友善」。

---

## 一句話總結

- **Python 版優勢**：工具數量更多、MFA 流程成熟、token 持久化已完成、Docker/China region/測試配套完整。
- **Rust 版優勢**：單一二進位部署、型別安全、session 共用設計更嚴謹、內建 cache + singleflight + rate limit、輸出更適合研究與資料分析。

也就是說，Python 版比較像「功能面最完整的原型與產品化入口」；Rust 版比較像「為長期穩定執行、研究工作流、以及 MCP/LLM 高頻呼叫場景重新設計的工程化版本」。

---

## 與 Python 版最核心的不同

| 面向 | Python 版 `garmin_mcp` | 我們的 Rust 版 |
|---|---|---|
| 執行方式 | 需要 Python + `uv`/套件環境 | 單一二進位，可直接執行 |
| 核心目標 | 盡量覆蓋 `python-garminconnect` 能力 | 針對 MCP 長時間使用情境做穩定化與工程化 |
| Tool 覆蓋 | README 標示 `96+ tools` | 目前 `73 tools`，覆蓋主要 Garmin Connect 功能 |
| 認證流程 | 已有 `garmin-mcp-auth`，支援 MFA 與 token 保存 | 目前依賴登入與 session file，MFA/持久化仍待補強 |
| Session 管理 | 以 Python library 與 token 機制為主 | 明確拆成 `Mutex<GarminClient>`、`RwLock<BearerToken>`、共用 `reqwest::Client` |
| 重複查詢 | 每個 tool 各自打 Garmin API | `moka` TTL cache + `singleflight` 合併重複請求 |
| 流量治理 | README/實作中未見統一 rate limit | `governor` 全域 rate limiter，GET/POST/DELETE 共用額度 |
| 研究輸出 | 以 JSON 為主 | JSON + CSV，且保留 EDF 擴充位 |
| 寫操作治理 | 依 Python Garmin client/requests 路徑 | 明確分開 GET 與 write path，寫入成功後會做 cache 失效 |
| 錯誤處理策略 | 多為例外字串回傳 | 我們額外做 Garmin account-gated endpoint 的友善訊息統一處理 |

---

## 我們比 Python 版更有優勢的地方

### 1. 單一二進位部署，對 MCP 使用者更省事

Python 版的優點是安裝容易、`uvx` 直接可跑，但本質上還是需要：

- Python runtime
- 套件相依
- `uv`/虛擬環境
- 某些情況下還要處理路徑、權限、環境隔離問題

我們的 Rust 版則是編譯完成後就只有一個可執行檔。對 MCP client 來說，這有幾個直接好處：

- 安裝與部署面更單純
- 不必擔心不同機器的 Python 環境差異
- 不需要額外建立/維護虛擬環境
- 在桌面端、研究機器、甚至未來容器化時都更容易標準化

這個優勢不一定會讓「第一次跑起來」比較快很多，但會讓**長期維運成本**明顯更低。

### 2. session 安全不是靠慣例，而是寫進型別與同步結構裡

這是我們最有價值、也最工程化的差異。

Garmin Connect 不是公開 API，而且對 session/token 的容忍度有限。MCP + LLM 的使用模式又很容易出現：

- 同一輪對話內重複問相似問題
- 多個 tool 在短時間內被連續呼叫
- 寫入與讀取交錯發生

Python 版主要是把同一個 `garmin_client` 分發給各模組使用；這在功能上可行，但在「高頻並發、避免重複 session、明確控制請求順序」這件事上，沒有像 Rust 版這麼明確的保證。

我們的 Rust 版做了三層同步設計：

- `Mutex<GarminClient>`：把 GET 路徑序列化，避免同一 session 被多個請求同時亂打
- `RwLock<BearerToken>`：讓寫入路徑可共享 token，必要時再獨佔 refresh
- 共用 `reqwest::Client`：避免每次寫操作都重新建立 HTTP client，能複用 TCP/TLS 連線池

這代表我們不是「希望不要出事」，而是把「不要重複建立 session、不要同時亂用 token、不要每次重新建連線」直接設計進架構裡。

### 3. 內建 cache + singleflight，更適合 LLM/MCP 真實使用情境

Python 版比較像是每個 tool 被呼叫時就直接打 Garmin API。這種做法在一般 CLI 腳本裡沒有問題，但在 MCP 場景裡，LLM 常常會：

- 用不同措辭重問同一件事
- 先查摘要、再查細節、再回頭查同一日期
- 平行觸發多個相近請求

我們的 Rust 版在 GET 路徑前面加了：

- `moka` TTL cache（目前 60 秒）
- `singleflight` 請求合併

好處很實際：

- 短時間內的重複問題不需要一直重打 Garmin
- 多個相同 key 的並發呼叫只會真的打一次 API
- cache hit 直接回傳，不會碰到 Mutex，也不會進網路層

這種設計非常適合「LLM 會重問、會追問、會 chain multiple tools」的使用模式，也是 Python 版目前沒有主打的能力。

### 4. 有全域 rate limiting，更不容易把 Garmin 打爆

Python 版 README 與公開實作強調的是工具覆蓋、MFA、token 流程，但沒有把「整體流量治理」當成核心功能。

我們的 Rust 版反而把這件事列為一級設計目標：

- `governor` token-bucket limiter
- GET / POST / PUT / DELETE 共用同一個 request budget
- 每次真正出網路前都會先過 limiter

這個優勢在平常手動點幾個工具時可能不明顯，但在 LLM 自動化、研究批量查詢、agent 多輪追問時差很多。  
**Rust 版更像是在保護 Garmin session 與保護帳號本身，而不只是把 API call 包起來。**

### 5. 輸出格式更適合研究與資料分析

Python 版確實有做不少「lightweight summary」來減少 context size，這點很好；但主要仍是回傳 JSON。

我們的 Rust 版進一步把輸出層抽象成 `ClinicalExport`，並對 10 個臨床/健康相關工具提供：

- `json`：適合 LLM 對話與快速閱讀
- `csv`：適合後續統計分析、資料整理、匯入 notebook / pandas / R
- EDF-ready trait slot：未來能往更正式的生理訊號格式延伸

這個差異代表我們不是只在乎「把資料拿回來」，而是更在乎「資料拿回來之後，要怎麼被研究者、分析流程、後續工具鏈真正使用」。

### 6. 寫入路徑分離得更清楚，也更接近可維運狀態

由於 `garmin_client` upstream 對 POST/PUT/DELETE 的支援有限，我們沒有硬把所有操作塞進同一條路，而是做了明確切分：

- GET：走 `garmin_client`
- 寫入操作：走共用 `reqwest::Client`
- 成功寫入後：`cache.invalidate_all()`，避免讀到舊資料

這個設計比「功能能用就好」更進一步，因為它處理的是**一致性（consistency）**。  
對 MCP 而言，如果你剛寫完 hydration / blood pressure / body composition，下一個讀取還拿到舊快取，那體驗其實很差。Rust 版已經把這種狀況納入設計。

### 7. 對 upstream bug 的掌控度更高

我們在開發過程中已經明確繞開 `garmin_client 0.2.1` 的多個問題，包括：

- URL double-slash 問題
- session token 被重新加引號
- upstream GET-only 限制
- 每次寫操作都建立新 `reqwest::Client` 的低效率問題

這些修正不只是「bug fix」，更代表我們對底層 Garmin 呼叫鏈的理解已經深入到可以主動補強，而不是單純依賴上游套件行為。

### 8. 對 account-gated / unsupported feature 的處理更適合給終端使用者

Garmin 很多端點不是每個帳號、每支手錶、每個地區都能用。Python 版通常會把錯誤或例外轉成字串回傳；這對開發者可接受，但對終端使用者或 LLM agent 不一定友善。

我們 Rust 版額外做了：

- `detect_garmin_error`
- `render_or_friendly`

讓某些 account-gated 工具在遇到 `404`、`NotAllowedException` 或空回應時，會回傳**可讀、可理解、可診斷**的友善訊息，而不是一段原始錯誤。  
這對 agent 來說尤其重要，因為它能比較穩定地判斷「是沒有資料、沒有功能、還是真的故障」。

### 9. 模組邊界更清楚，後續維護與擴充成本更低

Python 版是多模組拆分，這本身已經不錯；但 Rust 版在這之上，還有更明確的型別與 router 結構：

- 各 tool module 分檔
- `GarminMcpServer` 統一持有 `Arc<GarminApiClient>`
- `#[tool_router]` 集中註冊
- `serde` / `schemars` / `rmcp` 提供比較強的編譯期保證

長期來看，這會帶來兩個優勢：

- 新增工具時比較不容易破壞既有行為
- 重構內部資料流、輸出格式、共用錯誤處理時比較容易控管影響面

也就是說，Rust 版雖然目前 tool 數較少，但**底座更像是為長期演進而搭的**。

---

## 哪些情境下，我們的 Rust 版會比 Python 版更適合

### 適合 Rust 版的情境

- 你要把 Garmin MCP 當作長期穩定的桌面/本地 agent 能力
- 你在意部署簡化，不想依賴 Python runtime 與虛擬環境
- 你預期同一個 session 會被 LLM 反覆查詢、追問、連續使用
- 你在意高頻查詢時不要重複打 Garmin API
- 你需要把健康資料匯出成 CSV 做後續分析
- 你想在系統層面更嚴格地控制 request budget、token 使用與快取一致性

### 適合 Python 版的情境

- 你想要最完整的 tool 覆蓋面
- 你需要成熟的 MFA 流程
- 你需要 token persistence 與 pre-auth CLI
- 你要支援 `GARMIN_IS_CN`
- 你想直接沿用現有 Docker / pytest / Python 生態

---

## 必須誠實寫出的目前差距

如果這份文件要有說服力，就不能只寫 Rust 的優點，也要把目前還輸給 Python 版的地方講清楚。

### 1. 工具數量目前仍少於 Python 版

Python 版 README 標示 `96+ tools`，而我們目前是 `73 tools`。  
代表在功能覆蓋面上，Rust 版目前仍是「涵蓋主要功能，但還沒有追平全部廣度」。

### 2. MFA / 預先認證 / token persistence 目前仍是 Python 版更成熟

Python 版已經有：

- `garmin-mcp-auth`
- 互動式 MFA
- token 存放在 `~/.garminconnect`
- token 驗證與重新認證流程

Rust 版目前在這塊還沒有完全追上。這也是為什麼我們的優勢主要在**執行架構與工程品質**，而不是認證體驗。

### 3. Python 版在產品化配套上更完整

Python 版目前已公開提供：

- `uvx` 直接啟動方式
- Docker / Docker Compose
- China region 設定
- 完整測試敘述與測試分層

Rust 版目前雖然核心架構更強，但在這些周邊配套上還需要補齊，才能在「拿來就用」的產品成熟度上全面超越。

---

## 最適合放進對外文件的結論

如果要用一段話來描述我們這個 Rust 版本，最準確的說法不是「Rust 版功能比 Python 版更多」，因為這不是真的；更好的表述是：

> 我們的 `garmin_mcp_rust` 不是單純把 Python 版換語言重寫，而是針對 MCP/LLM 的真實使用模式重新做工程化設計：以單一二進位降低部署複雜度，以型別安全與同步模型強化 session 管理，以 cache + singleflight + rate limiting 控制 Garmin API 壓力，並把健康資料輸出提升到更適合研究與分析的格式。

再更精簡一點可以寫成：

> Python 版在功能廣度與認證體驗上更成熟；Rust 版則在部署、穩定性、session 安全、流量治理與研究輸出上更有工程優勢。

---

## 建議對外表述策略

如果這份 `advantages.md` 之後會拿來做展示、報告或 README 片段，建議採用這個口徑：

1. 先承認 Python 版是功能與概念來源，避免給人不客觀的感覺。
2. 明確指出 Rust 版不是追求「只多幾個 tool」，而是追求「更穩、更可部署、更適合 agent/研究場景」。
3. 把優勢聚焦在：
   - 單一二進位
   - 型別安全與 session safety
   - cache + singleflight
   - rate limiting
   - CSV/研究輸出
   - 友善錯誤處理
4. 同時誠實標註目前仍落後的地方：
   - tool coverage
   - MFA
   - token persistence
   - Docker / China region / 測試配套

這樣整體敘事會比單純說「Rust 比 Python 快、穩、強」更可信，也更像一個真正理解產品取捨的工程團隊。
