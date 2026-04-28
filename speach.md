# 講稿 — Garmin MCP (Rust)

---

## Slide 01 · Hero

大家好。今天我要介紹的是 Garmin MCP，一個用 Rust 寫的 Model Context Protocol 伺服器。

它的核心概念很簡單：你只需要一個靜態 binary，就能讓 Claude 直接讀取你的 Garmin Connect 健康數據。不需要 Python，不需要 venv，部署完就忘。

整個專案提供 77 個工具、橫跨 12 個模組，binary 本身約 10 MB。

---

## Slide 02 · Why Rust

先說為什麼要用 Rust 重寫。

原版是 Python 實作，功能沒有問題，但在幾個關鍵地方有隱患。

第一，部署複雜——需要 Python 3.12 加上 uv 或 pip。Rust 版本只需一個 binary，複製過去就能跑。

第二，記憶體佔用。

最重要的是**並發安全**。Python 版本在高並發下可能出現 race condition；Rust 版本的 `Mutex` 和 `RwLock` 是型別系統的一部分，compiler 會直接拒絕不安全的寫法。

另外原版沒有 rate limit——LLM 如果連續大量請求，Garmin 可能直接鎖掉你的帳號。我們加了 governor token bucket 來防止這個問題。

---

## Slide 03 · Architecture

這是 GET 請求的完整路徑，由上到下共六層。

最頂層是 **Tool Layer**，77 個工具透過 `#[tool_router]` 註冊到 `GarminMcpServer`，每個工具其實都只是 `Arc<GarminApiClient>` 的薄包裝。

往下是 **ClinicalExport**，負責把 API 回傳的資料整形成 JSON 或 CSV。

**moka Async Cache** 是最關鍵的一層。60 秒 TTL，最多 1000 筆。重點是它有 singleflight：多個工具同時問同一份資料，只會發出一次 HTTP 請求，其他人等結果。Cache hit 的話，連 Mutex 都不需要碰。

Cache miss 才會進到 **governor Rate Limiter**，整個系統的流量預算在這裡統一管控，讀寫都算。

**Rust Sync Layer** 確保同一時間只有一個 GET 能進到 Garmin client，並管理 bearer token 的讀寫鎖。

最底層才是真正打到 **Garmin Connect API** 的網路呼叫。

---

## Slide 04 · Concurrency Model

Rust 的並發安全不是靠紀律，是靠 type system 強制的。

這裡有三個核心 primitive。

`Mutex<GarminClient>` 負責 GET 請求的序列化。Garmin 的 client library 的 `api_request` 需要 `&mut self`，也就是說你根本無法同時有兩個 GET 在飛——Rust 型別系統不允許。

`RwLock<BearerToken>` 管理 bearer token。多個 POST 可以同時持有 read lock；token 需要 refresh 時才需要 write lock，用 double-checked locking 避免重複 refresh。

`reqwest::Client` 是一個共享的 HTTP client pool，TLS 握手和 TCP 連線都會被重複利用，不會每次工具呼叫都重新建連線。

下方的程式碼就是這個 struct 的完整定義——五個欄位，每個都有明確的所有權語意。

---

## Slide 05 · Cache & Rate Limit

快取設計有幾個數字值得記一下。

**60 秒 TTL** 是刻意選的。長到足以讓 LLM 在同一段對話裡重複問同樣的問題不會重打 API；但短到不會讓剛運動完的數據被舊快取蓋住。

**1000 筆**上限，LRU eviction，key 是把 endpoint 加上排序過的 query string 組成的字串，value 是 `Arc<Value>`——clone 只是 reference count 加一，不是深複製。

**60 req/min** 是對 Garmin 的流量預算。Garmin 沒有公開文件說明限制，這個數字是保守估計。重要的是讀跟寫共用同一個 limiter，不會因為讀多就把寫的額度擠掉。

**1 in-flight per key** 是 moka 的 singleflight 保證。十個工具同時問，只有一個去網路拿。

---

## Slide 06 · Security

Rust 的記憶體安全本身就是一種安全屬性。沒有 use-after-free，沒有 buffer overflow，這些是 C/C++ 專案需要審計的東西，在這裡 compiler 已經排除了。

但除了語言層面，我們在設計上也做了幾個決策。

**憑證**可以用 `_FILE` 模式，從 chmod 600 的檔案讀取 email 和密碼，不會出現在 Claude Desktop 的 JSON config 裡。

**Session 安全**：不可能意外建出第二個 OAuth session，型別系統拒絕這種寫法。

**攻擊面**：整個伺服器走 stdin/stdout，沒有 open port，外部無法連入。

**故意省略的工具**：`delete_activity` 不暴露出來——LLM 不應該有能力不可逆地刪你的訓練紀錄。`get_activity_details` 也省略了，因為它會回傳 50 到 500 KB 的 GPS 軌跡，不想讓精確位置資料流過 LLM context。

---

## Slide 07 · 77 Tools · 12 Modules

77 個工具分在 12 個模組裡，每個模組對應一個 Garmin Connect 的功能域。

數量最多的是 **Health & Wellness**，21 個工具，涵蓋睡眠、心率、壓力、body battery、HRV、SpO₂ 等。

**Activities** 有 14 個，可以抓特定日期的活動、心率區間分佈、訓練效果等。

特別值得一提的是 **Research** 模組，雖然只有 4 個工具，但它們支援最長 366 天的時序資料，一次呼叫就可以拿回一整年的 HRV 或睡眠資料，直接接 pandas 或 R 做分析。

另外有故意不做的兩個：`delete_activity` 因為不可逆所以省略，`get_activity_details` 因為 GPS 資料太大且敏感。

---

## Slide 08 · Research Output

這個模組是專門為研究者或想做自我量化分析的人設計的。

同一個工具可以回傳 JSON 或 CSV，由呼叫端透過 `format` 參數指定。

**JSON** 適合在對話裡讓 LLM 直接解讀，**CSV** 適合直接餵給 pandas 或 R。

舉個例子，右邊的 `get_daily_stats_range` 加上 `format: csv`，可以一次拿回 90 天、每天 20 個欄位的健康指標，直接 `pd.read_csv()` 就能開始分析。

`get_weekly_summary` 會幫你做 ISO week 的聚合，包含均值、標準差、最大最小值，省去自己 resample 的工夫。

未來計畫支援 EDF 格式，專門用來輸出生物訊號，trait slot 已經在 `output.rs` 留好了。

---

## Slide 09 · Closing

最後做個總結。

一個 binary。一個記憶體模型。一個藍色按鈕。

你不需要維護 Python 環境，不需要擔心 race condition，不需要半夜接到 Garmin 鎖帳號的警報。

安裝方式就一行：`cargo install --path .`，binary 放到 PATH，Claude Desktop config 加兩行 env，完成。

有問題歡迎提問，謝謝。

---
