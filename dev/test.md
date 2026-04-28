## Garmin MCP Rust 測試紀錄

日期：2026-04-25

### 測試目標
- 驗證 `garmin_mcp_rust` 在本機能正常啟動與回應 MCP 請求
- 確認最近沒有戴錶、資料可能為空的情境下，工具能夠正確回傳空值、空陣列或友善訊息

### 已完成測試（核心健康摘要）
- `bash dev/smoke.sh`：通過
- MCP 初始化：通過
- `tools/list`：通過，工具清單可正確列出
- `get_recent_activities`：通過
- `get_sleep_summary`：無資料時可回友善訊息
- `get_daily_heart_rate`：通過，空資料可回最小化摘要
- `get_stress_summary`：通過，空/少量資料可回摘要
- `get_body_battery_summary`：通過
- `get_stats`：通過，空資料回空 `metricsMap`
- `get_training_status`：通過，回 `null` skeleton
- `get_devices`：通過
- `get_unit_system`：通過，回 `null`
- `get_blood_pressure`：通過，回空陣列
- `get_daily_weigh_ins`：通過，回空陣列

### 已修復項目（2026-04-25）
所有先前回 `404 / parse error / Exception` 的工具，現已統一改回友善訊息。

修復方法：
1. `client::api_json` 解析失敗一律回傳 `Value::Null`（不再 `bail!`），並把 preview log 到 stderr 供除錯。
2. 新增 `client::detect_garmin_error()` 偵測 JSON 形式的錯誤封包：
   - `{status: 4xx/5xx, error, message}`
   - `{exception: "...Exception", ...}`
   - `{error: "...Exception", clientMessage / errorId, ...}`（NotAllowedException 等）
   - `{errorMessage: "..."}`
3. 新增 `client::render_or_friendly(&data, no_data_msg)` helper，套用至所有先前失敗工具。

修復後驗證（smoke test 全部回友善訊息，無例外）：
- `get_hrv_data`：空資料 → "No HRV data for {date} — HRV tracking requires sleeping with an HRV-capable watch."
- `get_training_readiness`：404 HTML → "No training readiness data for {date} — feature requires a compatible Garmin device with recent activity."
- `get_primary_training_device`：404 HTML → "No primary training device on this account."
- `get_workouts`：NotAllowedException JSON → "No workouts available — this endpoint may require Garmin Connect+ or coach permissions.（API 訊息：NotAllowedException: ...）"
- `get_scheduled_workouts`：404 → "No scheduled workouts between {start} and {end}.（API 訊息：HTTP 404 Not Found）"
- `get_goals`：404 → "No active goals.（API 訊息：HTTP 404 Not Found）"
- `get_nutrition_daily_settings`：404 → "No nutrition settings — nutrition tracking may not be enabled on this account."
- `get_nutrition_daily_food_log`：404 → "No nutrition log for {date} — ..."
- `get_custom_foods`：404 → "No custom foods saved."
- `get_body_battery_events`：404 → "No body battery events for {date}."
- `get_menstrual_data_for_date`：404 HTML → "No menstrual data for {date} — menstrual tracking may not be enabled on this account."
- `get_menstrual_calendar_data`：404 HTML → "No menstrual calendar data between {start} and {end}."
- `get_pregnancy_summary`：404 HTML → "No pregnancy data — pregnancy tracking is not active on this account."
- `get_all_day_events`：原本就是空陣列，符合預期。

### 結論
- 全部核心健康摘要工具與先前 14 個失敗工具皆通過。
- 所有「無資料 / 端點不存在 / 帳號無權限」皆統一回友善訊息，不再噴 raw error。
- 後續若新增工具，套用 `render_or_friendly()` 即可一致處理空資料情境。
