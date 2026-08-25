# Garmin MCP — Data Schema Reference

This is a live-verified reference for what each `garmin-mcp` tool actually returns, meant to make
writing analysis scripts against this server's output straightforward without re-deriving the shape
of every response by hand.

## Methodology

Every read-only tool (70 of 77) was exercised against a real, authenticated Garmin Connect account on
**2026-08-25**, via a one-off test harness (`tests/schema_dump.rs`, `cargo test --test schema_dump --
--ignored --nocapture`) that calls each tool function directly (the same functions the MCP tool layer
in `src/tools/mod.rs` wraps) and captures its raw string output. Date-scoped tools mostly used
yesterday / a 10-day / a 30-day window; see the test file for exact ranges.

The 7 write/mutating tools (`add_hydration_data`, `set_blood_pressure`, `add_body_composition`,
`add_gear_to_activity`, `remove_gear_from_activity`, `schedule_workout`, `delete_workout`) were
**deliberately not exercised live** — they would insert fake health data into, add a real calendar
entry to, or permanently delete data from a real account. Their request shape is documented from
source only.

The raw captured responses contain this account's real personal data (device serials, GPS
coordinates, health readings, account identifiers) and were kept local to the test run — they are
**not** included in this repository. Every schema below describes field names, types, and meaning
only; example values are either genuinely non-sensitive (activity type keys, HTTP status codes) or
synthetic.

### Status legend

| Symbol | Meaning |
|---|---|
| ✅ Working | Returned real JSON on this test account — schema below is observed, not guessed. |
| ⚠️ No data | The tool's own graceful "no data" message fired — expected behavior, not a bug. |
| ❌ Error | Garmin returned a raw HTTP error (404/403/405) and the tool surfaced it as error text rather than a friendly message. In most cases this is because the test account has simply never used that feature (badges, challenges, goals, gear, nutrition logging, menstrual/pregnancy tracking, advanced running-dynamics metrics on a lifestyle watch) — **not confirmed to be broken endpoints** unless called out below. |
| ⏭️ Not tested | Write/mutating tool, deliberately skipped; or a read tool whose required id (e.g. an activity/device/workout id) couldn't be resolved because the account has none. |

## Known issues found during this pass

1. **`user_profile.rs`'s 4 tools were calling stale/wrong Garmin endpoints — the strongest finding in
   this pass, not just account gating.** `src/auth.rs`'s own `resolve_display_name()` probes three
   candidate endpoints at startup of the *same* session used for this test run, and logs the outcome
   of each: `/userprofile-service/userprofile/v2/information` **404s**,
   `/userprofile-service/socialProfile` **returns 200** with a working `displayName` field, and
   `/userprofile-service/userprofile` **404s**. `get_full_name` and `get_unit_system` both called the
   confirmed-dead `v2/information` path; `get_user_profile` called the confirmed-dead
   `/userprofile-service/userprofile`; `get_userprofile_settings` called a fourth, never-probed path
   (`/userprofile-service/social-profile/settings`) that also 404s. **Fixed in this same change** — all
   four now use `/userprofile-service/socialProfile` or `/userprofile-service/userprofile/settings`
   (both confirmed working) and were re-verified live. See the User Profile section below.
2. **`get_workouts` returns HTTP 405 Method Not Allowed**, not 404 like everything else in this run —
   a 405 means Garmin recognizes the endpoint but rejected this request's verb/shape, which reads
   differently from "account has never used this feature." Worth re-checking against current Garmin
   Connect web traffic. **Not fixed** — out of scope for this pass, left for a follow-up.
3. **`all_day_events` returns HTTP 403 Forbidden**, the only 403 in this run (everything else is 404)
   — suggests a permission/entitlement gate rather than "no data," a different failure class from the
   rest of this document. **Not fixed.**
4. Several `activity_*` sub-resource tools (`activity_typed_splits`, `activity_split_summaries`,
   `activity_training_effect`) 404 for the one activity available on this account while
   `activity_splits` on the *same* activity works fine — plausibly this activity's type just lacks
   those sub-resources, not proof the endpoints are broken. **Not fixed** — insufficient evidence this
   is actually broken.

Items 2–4 were left as-is — this document only reports what live testing found for them. Item 1 had
strong enough in-repo evidence (a working endpoint already proven in the same codebase) to fix
directly; see `src/tools/user_profile.rs`.

## Coverage summary

33 tools returned real data, 4 returned a graceful "no data" message, 32 surfaced a raw Garmin HTTP
error, 1 read tool couldn't be tested (no workout on the account), and 7 write tools were skipped by
design. (The 4 `user_profile.rs` tools count as Working below — they were broken when first tested,
then fixed and re-verified live in this same change; see "Known issues" above.)

| Domain | Working | No data | Error | Not tested |
|---|---|---|---|---|
| Activities | 9 | 1 | 4 | 0 |
| Challenges, Badges & Goals | 2 | 0 | 6 | 0 |
| Devices & Gear | 3 | 0 | 4 | 2 |
| Health & Wellness | 10 | 3 | 8 | 0 |
| Nutrition / Training / Profile / Women's Health / Data Entry | 5 | 0 | 8 | 3 |
| Research | 4 | 0 | 0 | 0 |
| Workouts | 0 | 0 | 2 | 3 |
| **Total (77 tools)** | **33** | **4** | **32** | **8** |

---

## Activities

### `activities_by_date`
Get a list of activities between two dates, optionally filtered by activity type. Paginates the search endpoint internally (20/page) and merges all pages, then curates each activity to the same reduced shape as `recent_activities`.

**Params:** `start_date: string` — range start (YYYY-MM-DD) · `end_date: string` — range end (YYYY-MM-DD) · `activity_type: string, optional` — filter (e.g. "running", "cycling", "hiking")

**Live test:** ⚠️ No data — "No activities found between 2026-08-15 and 2026-08-25"

### `activity`
Get detailed summary for a specific activity by its Garmin activity ID. Returns the raw activity-service response (uncurated).

**Params:** `activity_id: string` — numeric Garmin activity ID

**Live test:** ✅ Working

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| accessControlRuleDTO | object | Privacy setting (`typeId: number`, `typeKey: string` e.g. "private") |
| activityId | number | Unique activity identifier |
| activityName | string | Activity title |
| activityTypeDTO | object | Type classification: `typeId` (number), `typeKey` (string, e.g. "hiking"), `parentTypeId` (number), `isHidden`/`restricted`/`trimmable` (boolean) |
| activityUUID | object | `{ uuid: string }` — globally unique activity identifier |
| eventTypeDTO | object | Event category: `typeId` (number), `typeKey` (string, e.g. "uncategorized"), `sortOrder` (number) |
| isMultiSportParent | boolean | True if this activity is a container for child multisport legs |
| locationName | string | Human-readable start location |
| metadataDTO | object | Device/upload metadata — see below |
| summaryDTO | object | Performance metrics — see below |
| timeZoneUnitDTO | object | `timeZone`/`unitKey` (string, IANA zone e.g. "Asia/Taipei"), `unitId` (number), `factor` (number) |
| userProfileId | number | Garmin user profile ID that owns the activity |

`metadataDTO` (notable fields; many others are device/eBike/dive fields that are `null` unless applicable):
| Field | Type | Meaning |
|---|---|---|
| deviceApplicationInstallationId | number | Installed device-app instance ID |
| deviceMetaDataDTO | object | Recording device identifiers (`deviceId`, `deviceTypePk`, `deviceVersionPk`) |
| fileFormat | object | Source file format, `{ formatId: number, formatKey: string }` e.g. "fit" |
| hasPolyline / hasHeatMap / hasSplits / hasHrTimeInZones / hasChartData / hasIntensityIntervals / hasPowerTimeInZones | boolean | Data-availability flags telling clients which sub-resources exist for this activity |
| lapCount | number | Number of laps recorded |
| manualActivity | boolean | True if manually entered rather than device-recorded |
| manufacturer | string | Recording device manufacturer, e.g. "GARMIN" |
| personalRecord | boolean | True if this activity set a personal record |
| favorite | boolean | User-starred flag |
| elevationCorrected | boolean | Whether elevation was corrected via a digital elevation model |
| uploadedDate / lastUpdateDate | string | ISO timestamps for upload / last edit |
| userInfoDto | object | Uploader's display name, profile image URLs, profile ID |
| activityImages | array | Attached photos (empty if none) |
| videoUrl, associatedCourseId, associatedWorkoutId, diveNumber, calendarEventInfo, sensors, eBike* fields | various | Null unless the activity has course/workout linkage, dive-computer data, or e-bike telemetry |

`summaryDTO` (performance metrics):
| Field | Type | Meaning |
|---|---|---|
| startTimeLocal / startTimeGMT | string | Activity start timestamp, local and UTC |
| distance | number | Total distance in meters |
| duration / elapsedDuration / movingDuration | number | Duration in seconds — active, wall-clock, and moving-only |
| calories | number | Calories burned |
| bmrCalories | number | Basal-metabolic portion of calories attributed to the activity |
| averageHR / maxHR / minHR | number | Heart rate in bpm |
| averageSpeed / maxSpeed / averageMovingSpeed | number | Speed in m/s |
| averageRunCadence / maxRunCadence | number | Steps per minute |
| steps | number | Total steps |
| elevationGain / elevationLoss | number | Cumulative ascent/descent in meters |
| avgElevation / maxElevation / minElevation | number | Elevation in meters |
| maxVerticalSpeed | number | Vertical speed in m/s |
| startLatitude / startLongitude / endLatitude / endLongitude | number | GPS coordinates in degrees |
| trainingEffectLabel | string | Dominant training-effect category, e.g. "AEROBIC_BASE" |
| aerobicTrainingEffectMessage / anaerobicTrainingEffectMessage | string | Coded training-effect descriptions |
| moderateIntensityMinutes / vigorousIntensityMinutes | number | Minutes spent at each intensity level |
| differenceBodyBattery | number | Body Battery change over the course of the activity |
| waterEstimated | number | Estimated fluid loss (mL) |
| minActivityLapDuration | number | Shortest lap duration in seconds |

### `recent_activities`
Get the N most recent activities, curated to a compact field set.

**Params:** `limit: u32, optional` — number of activities to return, clamped 1–50, default 10

**Live test:** ✅ Working

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| count | number | Number of activities returned |
| activities | array | Array of curated activity objects, each: |

Curated activity object (fields are omitted entirely when the source value was null/absent, so not every object has every key):
| Field | Type | Meaning |
|---|---|---|
| id | number | Garmin activity ID |
| name | string | Activity title |
| type | string | Activity type key, e.g. "running" |
| start_time | string | Local start time, "YYYY-MM-DD HH:MM:SS" |
| distance_meters | number | Distance in meters |
| duration_seconds | number | Duration in seconds |
| calories | number | Calories burned |
| avg_hr_bpm | number | Average heart rate in bpm |
| max_hr_bpm | number | Max heart rate in bpm |
| steps | number | Step count (present for walking/running/hiking-type activities) |

`activities_by_date` above returns the same curated element shape, wrapped in `{ count, date_range: { start, end }, activities }`.

### `activities_fordate`
Get activities that occurred on a specific date, via Garmin's dedicated by-date endpoint (distinct from `activities_by_date`'s search endpoint).

**Params:** `date: string` — YYYY-MM-DD

**Live test:** ❌ HTTP 404 Not Found — queried date (2026-08-24) had zero activities for this account; the dump shows Garmin's raw 404 error envelope surfacing through as tool error text rather than a friendly "no activities" message. Plausibly this endpoint 404s on empty days rather than being broken — not confirmed a bug.

### `activity_splits`
Get lap/interval splits for a specific activity.

**Params:** `activity_id: string` — numeric Garmin activity ID

**Live test:** ✅ Working

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| activityId | number | Garmin activity ID |
| eventDTOs | array | Timer/lap-trigger events, each: `sectionTypeDTO` (object: `id` number, `key`/`sectionTypeKey` string e.g. "TIMER_TRIGGER"), `startTimeGMT` (string), `startTimeGMTDoubleValue` (number, epoch milliseconds) |
| lapDTOs | array | One object per lap/interval — see below |

`lapDTOs` element:
| Field | Type | Meaning |
|---|---|---|
| lapIndex | number | 1-based lap number |
| messageIndex | number | FIT-file message index for the lap |
| intensityType | string | e.g. "ACTIVE" |
| startTimeGMT | string | Lap start timestamp (UTC) |
| duration / elapsedDuration / movingDuration | number | Lap duration in seconds (active/wall-clock/moving-only) |
| distance | number | Lap distance in meters |
| calories / bmrCalories | number | Calories burned / basal-metabolic portion, for the lap |
| averageHR / maxHR | number | Heart rate in bpm |
| averageSpeed / maxSpeed / averageMovingSpeed | number | Speed in m/s |
| averageRunCadence / maxRunCadence | number | Steps per minute |
| elevationGain / elevationLoss / maxElevation / minElevation | number | Elevation metrics in meters |
| maxVerticalSpeed | number | Vertical speed in m/s |
| startLatitude / startLongitude / endLatitude / endLongitude | number | GPS coordinates in degrees |
| connectIQMeasurement | array | Custom Connect IQ app measurements (empty unless present) |
| lengthDTOs | array | Pool-length breakdown for swim activities (empty for non-swim) |

### `activity_typed_splits`
Get typed splits (by segment type) for a specific activity.

**Params:** `activity_id: string` — numeric Garmin activity ID

**Live test:** ❌ HTTP 404 Not Found — same activity as `activity_splits` above (which succeeded); this sub-resource may simply not exist for a hiking-type activity rather than indicating a broken endpoint.

### `activity_split_summaries`
Get split summary statistics for a specific activity.

**Params:** `activity_id: string` — numeric Garmin activity ID

**Live test:** ❌ HTTP 404 Not Found — same activity tested; likely no split-summary data for this activity/type, root cause not confirmed.

### `activity_weather`
Get weather conditions recorded during a specific activity.

**Params:** `activity_id: string` — numeric Garmin activity ID

**Live test:** ✅ Working

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| apparentTemp | number | "Feels like" temperature at the activity location (unit not specified in payload) |
| temp | number | Ambient temperature |
| dewPoint | number | Dew point temperature |
| relativeHumidity | number | Relative humidity, percent |
| issueDate | string | ISO timestamp the weather observation was issued |
| latitude / longitude | number | Coordinates of the reporting weather station |
| weatherStationDTO | object | Reporting station: `id` (string), `name` (string), `timezone` (string or null) |
| weatherTypeDTO | object | Condition classification: `desc` (string, e.g. "Unknown"), `image` (string or null), `weatherTypePk` (number or null) |
| windDirection | number | Wind direction in degrees |
| windDirectionCompassPoint | string | Compass abbreviation, e.g. "e" |
| windSpeed | number | Wind speed |
| windGust | number or null | Wind gust speed; null if not recorded |

### `activity_hr_in_timezones`
Get heart rate time-in-zone breakdown for a specific activity.

**Params:** `activity_id: string` — numeric Garmin activity ID

**Live test:** ✅ Working

**Response schema:** Array of 5 objects (one per HR zone), each:
| Field | Type | Meaning |
|---|---|---|
| zoneNumber | number | HR zone index, 1–5 |
| zoneLowBoundary | number | Lower bound of the zone in bpm |
| secsInZone | number | Seconds spent in this zone during the activity |

### `activity_exercise_sets`
Get exercise sets (strength/gym) for a specific activity.

**Params:** `activity_id: string` — numeric Garmin activity ID

**Live test:** ✅ Working — but returns no set data for this activity (a hike, not a strength workout)

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| activityId | number | Garmin activity ID |
| exerciseSets | null (array when present) | Strength-training set data; null when the activity type doesn't record sets (would be an array of set objects for a strength-training activity) |

### `activity_gear`
Get the gear (shoes, bike, etc.) associated with a specific activity.

**Params:** `activity_id: string` — numeric Garmin activity ID

**Live test:** ✅ Working — response body is exactly `[]` (empty JSON array); this activity had no gear tagged, so no element schema could be observed from this run.

### `activity_training_effect`
Get training effect (aerobic and anaerobic) for a specific activity.

**Params:** `activity_id: string` — numeric Garmin activity ID

**Live test:** ❌ HTTP 404 Not Found — same activity tested; note `activity`'s `summaryDTO` already carries `aerobicTrainingEffectMessage`/`anaerobicTrainingEffectMessage`/`trainingEffectLabel`, so this may be a legacy/unpopulated sub-resource for this activity rather than a broken endpoint.

### `count_activities`
Get the total activity count across all time.

**Params:** none

**Live test:** ✅ Working

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| totalCount | number | Total activities on the account, all time |
| nonMultisportCount | number | Count of standalone (non-multisport) activities |
| multisportParentCount | number | Count of multisport parent activities (e.g. triathlons) |
| multisportChildCount | number | Count of child legs within multisport activities |

### `activity_types`
Get the list of all Garmin activity types with their keys and IDs.

**Params:** none

**Live test:** ✅ Working — response is an array of ~153 objects covering every Garmin activity type

**Response schema:** Array of ~153 objects, each:
| Field | Type | Meaning |
|---|---|---|
| typeId | number | Numeric activity type identifier |
| typeKey | string | Machine-readable type key, e.g. "running", "cycling", "hiking" |
| parentTypeId | number | `typeId` of this type's parent/category |
| isHidden | boolean | Whether this type is hidden from normal UI pickers |
| restricted | boolean | Whether the type is restricted |
| trimmable | boolean | Whether GPS start/end trimming is supported for this type |

---

## Challenges, Badges & Goals

### `earned_badges`
Get all badges earned by the user.

**Params:** none

**Live test:** ❌ HTTP 404 Not Found — `GET /badge-service/badge/earned/{display_name}` returned a 404 JSON error envelope (`{"timestamp","status","error","path"}`). This is a user-scoped resource; every user-scoped badge/challenge/goal endpoint in this domain 404s for this account while the public `available_badge_challenges` list works fine, most likely because this account has simply never earned a badge (Garmin may lazily provision these per-user resources rather than this being a bug).

### `available_badge_challenges`
Get available badge challenges the user can join.

**Params:** none

**Live test:** ✅ Working — returns a large public catalog (no per-user scoping in the request).

**Response schema:**
Array of ~297 objects, each describing one badge/challenge definition:
| Field | Type | Meaning |
|---|---|---|
| badgeAssocDataId | number \| null | Id of an associated entity (e.g. a linked event), when applicable |
| badgeAssocDataName | string \| null | Name of the associated entity |
| badgeAssocType | string | Association category, e.g. `"none"` |
| badgeAssocTypeId | number | Numeric code for `badgeAssocType` |
| badgeCategoryId | number | Numeric category the badge belongs to (e.g. cycling, running, streaks) |
| badgeChallengeStatusId | number \| null | Status code if this badge is tied to an active challenge |
| badgeDifficultyId | number | Numeric difficulty tier |
| badgeEarnedDate | string \| null | ISO timestamp the current user earned it, if earned |
| badgeEarnedNumber | number \| null | How many times the user has earned this badge (for repeatable badges) |
| badgeEndDate | string | ISO timestamp the challenge window closes |
| badgeId | number | Unique numeric badge identifier |
| badgeIsViewed | boolean \| null | Whether the user has viewed this badge in the app |
| badgeKey | string | Machine-readable slug identifying the badge |
| badgeLimitCount | number \| null | Max number of times the badge can be earned, if limited |
| badgeName | string | Human-readable badge/challenge title |
| badgePoints | number | Point value awarded for earning the badge |
| badgeProgressValue | number \| null | Current progress toward the target, for the requesting user |
| badgePromotionCodeTypeList | array | List of promotional code types tied to the badge (often empty) |
| badgeSeriesId | number \| null | Id grouping this badge into a recurring series |
| badgeStartDate | string | ISO timestamp the challenge window opens |
| badgeTargetValue | number | Numeric goal to complete the badge (unit implied by `badgeUnitId`) |
| badgeTypeIds | array of number | Numeric tags classifying the badge (activity type, category, etc.) |
| badgeUnitId | number | Numeric unit code for `badgeTargetValue`/`badgeProgressValue` (e.g. meters) |
| badgeUuid | string | UUID identifying the badge definition |
| createDate | string \| null | ISO timestamp the badge record was created |
| currentPlayerType | string \| null | Type of the requesting player context, if applicable |
| displayName | string \| null | Display name of the earning user, when scoped to a user |
| earnedByMe | boolean | Whether the requesting account has earned this badge |
| fullName | string \| null | Full name of the earning user, when scoped to a user |
| premium | boolean | Whether the badge requires a Garmin Connect+ / premium subscription |
| promotionCodeStatus | string \| null | Status of any associated promo code |
| relatedBadges | array \| null | Related badge definitions, if any |
| reprocessable | boolean | Whether progress toward this badge can be recalculated/reprocessed |
| userJoined | boolean \| null | Whether the user has joined this challenge |
| userProfileId | number \| null | Numeric profile id of the earning user, when scoped to a user |

### `badge_challenges`
Get badge challenges the user has joined.

**Params:** none

**Live test:** ❌ HTTP 404 Not Found — `GET /badge-service/badge/challenges/{display_name}` returned the same JSON error envelope as `earned_badges`. Likely explanation: this account has never joined a badge challenge.

### `non_completed_badge_challenges`
Get badge challenges the user has not yet completed.

**Params:** none

**Live test:** ❌ HTTP 404 Not Found — `GET /badge-service/badge/challenges/non-completed/{display_name}` returned the same JSON error envelope. Likely explanation: same as above, no joined-but-incomplete challenges to lazily provision.

### `adhoc_challenges`
Get ad-hoc fitness challenges (supports pagination via start/limit).

**Params:** `start: u32` — pagination offset, default 0 · `limit: u32` — page size, default 20, server-side clamped to 1–100

**Live test:** ❌ HTTP 404 Not Found — `GET /fitnesschallenge-service/challenge/adhoc` returned an HTML "Page Not Found" body rather than JSON (a different failure shape than the badge-service 404s, but the same underlying likely cause: no ad-hoc challenges for this account).

### `inprogress_virtual_challenges`
Get virtual challenges currently in progress.

**Params:** none

**Live test:** ❌ HTTP 404 Not Found — `GET /fitnesschallenge-service/challenge/virtual/in-progress` returned the same HTML "Page Not Found" body. Likely explanation: this account is not enrolled in any virtual challenge.

### `goals`
Get active user goals, optionally filtered by goal type.

**Params:** `goal_type: Option<String>` — optional filter passed through as the API's `goalType` query param (e.g. "step", "calories", "distance"); omitted when empty/unset

**Live test:** ❌ HTTP 404 Not Found — `GET /goal-service/goal/list?status=active&start=1&limit=30&sortOrder=asc` returned a JSON error envelope. Likely explanation: this account has no goals configured, so the per-user goal-list resource was never provisioned. Note this tool otherwise uses `render_or_friendly` with a "No active goals." fallback message for the empty-but-present case — that path wasn't exercised here because the endpoint itself 404s.

### `personal_records`
Get personal records (PRs) across all activity types.

**Params:** none

**Live test:** ✅ Working

**Response schema:**
Array of objects, each one personal-record entry:
| Field | Type | Meaning |
|---|---|---|
| actStartDateTimeInGMTFormatted | string \| null | Formatted GMT start time of the activity that set the PR, if linked to one |
| activityId | number | Id of the activity that set the record; 0 when not linked to a specific activity |
| activityName | string \| null | Name of the linked activity |
| activityStartDateTimeInGMT | string \| null | Raw GMT start timestamp of the linked activity |
| activityStartDateTimeLocal | string \| null | Raw local start timestamp of the linked activity |
| activityStartDateTimeLocalFormatted | string \| null | Formatted local start timestamp of the linked activity |
| activityType | string \| null | Activity type key of the linked activity, e.g. "running" |
| id | number | Unique identifier for this PR record |
| poolLengthUnit | string \| null | Pool length unit for swimming PRs (e.g. "yard"/"meter"), null otherwise |
| prStartTimeGmt | number | Epoch milliseconds (GMT) the record was set |
| prStartTimeGmtFormatted | string | ISO-formatted GMT timestamp the record was set |
| prStartTimeLocal | number | Epoch milliseconds (local) the record was set |
| prStartTimeLocalFormatted | string | ISO-formatted local timestamp the record was set |
| prTypeLabelKey | string \| null | Localization key for the PR type label, when provided |
| status | string | Record status, e.g. "ACCEPTED" |
| typeId | number | Numeric code identifying which PR category this is (e.g. longest run, fastest time over a distance, max elevation gain); Garmin does not include a human label in this response |
| value | number | The record value; unit is implied by `typeId` (e.g. meters for distance records, seconds for time records) |

---

## Devices & Gear

### `get_devices`
Lists all Garmin devices registered to the account.

**Params:** none

**Live test:** ✅ Working

**Response schema:**
Array of device objects (this account returned 1), each an object with ~311 fields. Key identifying fields:

| Field | Type | Meaning |
|---|---|---|
| `deviceId` | number | Unique ID for this device registration |
| `unitId` | number | Device unit ID, typically equal to `deviceId` |
| `applicationKey` | string | Internal Garmin Connect model key (e.g. `"venu3"`) |
| `productSku` / `actualProductSku` | string | Garmin product SKU / part number |
| `partNumber` | string | Hardware part number |
| `deviceTypeSimpleName` | string | Human-readable model name (e.g. `"Garmin Venu 3"`) |
| `displayName` / `productDisplayName` | string | Display name shown in Garmin Connect (e.g. `"Venu 3"`) |
| `serialNumber` | string | Device serial number |
| `deviceStatus` | string | Lifecycle status (e.g. `"active"`) |
| `deviceCategories` | array of string | Feature categories the device belongs to (e.g. `["FITNESS","WELLNESS","GOLF"]`) |
| `currentFirmwareVersion` | string | Installed firmware version string |
| `currentFirmwareVersionMajor` / `Minor` | number | Parsed firmware version components |
| `registeredDate` | number | Epoch-ms timestamp when the device was registered to the account |
| `imageUrl` | string | URL to a product image |
| `primary` | boolean | Whether this is the account's primary device |
| `maxWorkoutCount` | number | Max structured workouts the device can store |
| `supportedHrZones` | array of string | HR zone types the device supports (e.g. `["RUNNING","CYCLING","ALL"]`) |
| `incompatibleApplications` | array of string | Garmin Connect app modes incompatible with this device |

...plus ~236 additional boolean `*Capable` feature-flag fields (one per hardware/software capability, e.g. `bodyBatteryCapable`, `solarChargeCapable`, `hrvStatusCapable`), and a further ~25 non-`Capable` boolean device-attribute fields (e.g. `wifi`, `hybrid`, `wellness`, `weightScale`, `hasOpticalHeartRate`) describing hardware traits or account roles rather than feature toggles.

### `get_device_last_used`
Returns the device most recently synced/used on the account.

**Params:** none

**Live test:** ✅ Working — structurally valid JSON, but every field was `null` for this account (no last-used-device record populated)

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| `applicationId` | null (observed) | ID of the last-used Connect application; presumably a number once populated |
| `applicationVersionId` | null (observed) | Version identifier of that application |
| `deviceId` | null (observed) | ID of the most recently used device |
| `deviceStatus` | null (observed) | Status of that device at last use |
| `lastDownloadTimestamp` | null (observed) | Epoch-ms timestamp of last data download from the device |
| `lastUploadTimestamp` | null (observed) | Epoch-ms timestamp of last data upload to the device |
| `userId` | null (observed) | Account user ID tied to the last-used record |

### `get_device_settings`
Returns the full settings/info payload for one specific device.

**Params:** `device_id: string` — Garmin device ID (the `deviceId`/`unitId` value from `get_devices`)

**Live test:** ✅ Working

**Response schema:**
A single object (not an array) with ~67 top-level fields — genuinely a different, smaller shape than `get_devices`, not just a subset: only 7 `*Capable` flags (vs. ~236), and it adds a nested `baseDeviceDTO` summary object plus a `softwareUpdates` array not present in `get_devices`.

| Field | Type | Meaning |
|---|---|---|
| `deviceId` | number | Unique device identifier |
| `unitId` | number | Device unit ID |
| `applicationKey` | string | Internal Garmin Connect model key |
| `productSku` | string | Garmin product SKU |
| `partNumber` | string | Hardware part number |
| `displayName` / `productDisplayName` | string | Display name shown in Garmin Connect |
| `serialNumber` | string | Device serial number |
| `deviceStatus` | string | Lifecycle status (e.g. `"active"`) |
| `deviceCategories` | array of string | Feature categories the device belongs to |
| `currentFirmwareVersion` | string | Installed firmware version string |
| `createdTs` / `updatedTs` | string (ISO 8601) | Record creation / last-update timestamps |
| `imageUrl` | string | URL to a product image |
| `primary` | boolean | Whether this is the account's primary device |
| `baseDeviceDTO` | object | Condensed duplicate of core identity fields: `deviceId`, `serialNumber`, `unitId`, `displayName`, `currentFirmwareVersion`, `primaryActivityTrackerIndicator`, `wifiSetup` |
| `softwareUpdates` | array of objects | Pending firmware/content downloads queued for the device. Each entry: `applicationKey`, `firmwareVersion`, `messageId` (number), `messageStatus`, `messageType`, and a nested `metaData` object (`fileName`, `fileSize`, `productName`, `version`, `deliveryRestrictions`, download server paths) |

...plus 7 boolean `*Capable` fields (`cellularCapable`, `customIntensityMinutesCapable`, `floorsClimbedGoalCapable`, `hrZonesCapable`, `intensityMinutesGoalCapable`, `moderateIntensityMinutesGoalCapable`, `otaCapable`), and ~15 further non-`Capable` boolean device-attribute fields (e.g. `wifi`, `hybrid`, `wellness`, `weightScale`, `hasOpticalHeartRate`).

### `get_primary_training_device`
Returns the device designated as the account's primary training device.

**Params:** none

**Live test:** ❌ HTTP 404 Not Found

### `get_device_solar_data`
Returns solar-charging data for a device.

**Params:** `device_id: string` — Garmin device ID

**Live test:** ❌ HTTP 404 Not Found — plausible: this account's device (`solarChargeCapable: false` per `get_devices`) has no solar-charging hardware

### `get_device_alarms`
Returns configured alarms for a device.

**Params:** `device_id: string` — Garmin device ID

**Live test:** ❌ HTTP 404 Not Found

### `get_gear`
Lists gear (shoes, bikes, etc.) configured on the account.

**Params:** none (resolves the account's display name internally via `require_display_name` and queries by it)

**Live test:** ❌ HTTP 404 Not Found — plausible: no gear configured on this account

### `add_gear_to_activity`
Associates a gear item with an activity.

**Params:**
- `gear_uuid: string` — UUID of the gear item to attach
- `activity_id: string` — Garmin activity ID to attach the gear to

**Live test:** ⏭️ Not tested (write tool, deliberately not exercised to avoid mutating a real activity)

Request shape (from source): `POST /gear-service/gear/{gear_uuid}/activity/{activity_id}` with an empty JSON body (`null`). Response is passed through as pretty-printed JSON on success.

### `remove_gear_from_activity`
Removes a gear-to-activity association.

**Params:**
- `gear_uuid: string` — UUID of the gear item to detach
- `activity_id: string` — Garmin activity ID to detach the gear from

**Live test:** ⏭️ Not tested (write tool, deliberately not exercised)

Request shape (from source): `DELETE /gear-service/gear/{gear_uuid}/activity/{activity_id}`. On success the tool returns a fixed confirmation string (`"Gear {gear_uuid} removed from activity {activity_id}"`), not Garmin's response body.

---

## Health & Wellness

### `stats`
Daily health stats: steps, calories, heart rate, stress levels, body battery, SpO2, and intensity minutes.

**Params:** `date: string` — YYYY-MM-DD · `format: "json" | "csv"` — optional, defaults to JSON

**Live test:** ✅ Working

**Response schema:** `FlatSummary` shape (also accepts `format: "csv"`, which renders a single header row + single value row). Fields are curated: pulled from Garmin's raw daily-summary payload via a fixed src→dst rename table (`STATS_FIELDS`), so the names below are already server-side renames, not Garmin's originals. Notable renames: `totalDistanceMeters`→`distance_meters`, `bodyBatteryMostRecentValue`→`body_battery_current`, `lastSevenDaysAvgRestingHeartRate`→`last_7_days_avg_resting_hr`, `averageSpo2`→`avg_spo2_percent`, `stressQualifier`→`stress_qualifier`. Any source field absent or null in Garmin's response is simply omitted from the output map.

| Field | Type | Meaning |
|---|---|---|
| date | string | Calendar date (YYYY-MM-DD) |
| total_steps | number | Total steps for the day |
| daily_step_goal | number | Step goal for the day |
| distance_meters | number | Total distance covered, in meters |
| total_calories | number | Total calories burned |
| active_calories | number | Calories burned from activity (above BMR) |
| bmr_calories | number | Basal metabolic rate calories |
| highly_active_seconds | number | Seconds spent in highly-active intensity |
| active_seconds | number | Seconds spent active |
| sedentary_seconds | number | Seconds spent sedentary |
| moderate_intensity_minutes | number | Minutes at moderate exercise intensity |
| vigorous_intensity_minutes | number | Minutes at vigorous exercise intensity |
| intensity_minutes_goal | number | Weekly/daily intensity-minutes goal |
| min_heart_rate_bpm | number | Minimum heart rate for the day |
| max_heart_rate_bpm | number | Maximum heart rate for the day |
| resting_heart_rate_bpm | number | Resting heart rate |
| last_7_days_avg_resting_hr | number | 7-day trailing average resting HR |
| avg_stress_level | number | Average stress score (0-100 scale) |
| max_stress_level | number | Peak stress score for the day |
| stress_qualifier | string | Garmin's categorical label for the day's stress pattern, e.g. `"BALANCED_AWAKE"` |
| body_battery_charged | number | Body Battery points gained during the day |
| body_battery_drained | number | Body Battery points spent during the day |
| body_battery_highest | number | Highest Body Battery value reached |
| body_battery_lowest | number | Lowest Body Battery value reached |
| body_battery_current | number | Most recent Body Battery reading |
| avg_spo2_percent | number | Average blood-oxygen saturation, % |
| lowest_spo2_percent | number | Lowest blood-oxygen saturation, % |
| avg_waking_respiration | number | Average waking breathing rate, breaths/min |
| highest_respiration | number | Highest breathing rate recorded, breaths/min |
| lowest_respiration | number | Lowest breathing rate recorded, breaths/min |

### `sleep_summary`
Curated sleep summary for a date: total duration, sleep stages, respiration, SpO2, and sleep scores.

**Params:** `date: string` — YYYY-MM-DD · `format: "json" | "csv"` — optional, defaults to JSON

**Live test:** ⚠️ No data — "No sleep data for 2026-08-24 — watch may not have been worn that night." (Fires when the curated field map yields ≤1 populated field from Garmin's `dailySleepDTO`.)

### `daily_heart_rate`
Daily heart rate summary: resting/min/max HR and sample count.

**Params:** `date: string` — YYYY-MM-DD · `format: "json" | "csv"` — optional, defaults to JSON

**Live test:** ✅ Working

**Response schema:** `FlatSummary` shape (also accepts `format: "csv"`, single header + value row). Curated from Garmin's `dailyHeartRate` endpoint; the raw per-sample `heartRateValues` array is not included, only its count.

| Field | Type | Meaning |
|---|---|---|
| date | string | Calendar date (YYYY-MM-DD) |
| resting_heart_rate_bpm | number | Resting heart rate |
| max_heart_rate_bpm | number | Maximum heart rate for the day |
| min_heart_rate_bpm | number | Minimum heart rate for the day |
| last_7_days_avg_resting_hr | number | 7-day trailing average resting HR |
| start_local | string | Start of the measurement window, local time (ISO-like timestamp) |
| end_local | string | End of the measurement window, local time |
| heart_rate_sample_count | number | Count of raw HR samples Garmin recorded that day (derived, not a renamed source field) |

### `stress_summary`
Daily stress summary: average and max stress levels, with sample counts.

**Params:** `date: string` — YYYY-MM-DD · `format: "json" | "csv"` — optional, defaults to JSON

**Live test:** ✅ Working

**Response schema:** `FlatSummary` shape (also accepts `format: "csv"`, single header + value row). Curated from Garmin's `dailyStress` endpoint; the raw per-sample `stressValuesArray` / `bodyBatteryValuesArray` arrays are not included, only their counts.

| Field | Type | Meaning |
|---|---|---|
| date | string | Calendar date (YYYY-MM-DD) |
| avg_stress_level | number | Average stress score (0-100 scale) |
| max_stress_level | number | Peak stress score for the day |
| stress_chart_offset | number | Chart-rendering offset value used by Garmin's own UI (internal plotting hint) |
| stress_chart_y_origin | number | Chart Y-axis origin value used by Garmin's own UI |
| start_local | string | Start of the measurement window, local time |
| end_local | string | End of the measurement window, local time |
| stress_sample_count | number | Count of raw stress samples in the day (derived) |
| body_battery_sample_count | number | Count of raw body-battery samples in the day (derived) |

### `body_battery_summary`
Body battery summary for a date: charged, drained, starting/ending values, sample count.

**Params:** `date: string` — YYYY-MM-DD · `format: "json" | "csv"` — optional, defaults to JSON

**Live test:** ✅ Working

**Response schema:** `TimeseriesArray` shape — summary fields plus `sample_count` and first/last sample timestamps, not the raw per-sample array itself. `format: "csv"` instead emits the raw samples as `timestamp_ms,body_battery` rows (one row per sample).

| Field | Type | Meaning |
|---|---|---|
| date | string | Calendar date (YYYY-MM-DD) |
| charged | number | Body Battery points gained during the day |
| drained | number | Body Battery points lost during the day |
| starting_value | number | Body Battery value at the first sample of the day |
| ending_value | number | Body Battery value at the last sample of the day |
| sample_count | number | Number of `[timestamp, value]` samples Garmin returned for the day |
| first_sample_ts_ms | number | Epoch milliseconds of the first sample |
| last_sample_ts_ms | number | Epoch milliseconds of the last sample |

### `daily_steps`
Daily step counts for each day in a date range.

**Params:** `start_date: string` — YYYY-MM-DD · `end_date: string` — YYYY-MM-DD

**Live test:** ✅ Working

**Response schema:** Raw, un-curated Garmin JSON (passed through `serde_json::to_string_pretty`, not run through `FlatSummary`/`ClinicalExport`) — array of one object per day in range, in Garmin's own field naming.

Array of N objects, each:

| Field | Type | Meaning |
|---|---|---|
| calendarDate | string | Calendar date (YYYY-MM-DD) |
| stepGoal | number | Step goal for that day |
| totalDistance | number \| null | Total distance covered that day, meters; `null` for days with no recorded data (e.g. future dates, or days the device wasn't synced) |
| totalSteps | number \| null | Total steps for that day; `null` under the same conditions as `totalDistance` |

### `training_readiness`
Training readiness score and contributing factors for a date.

**Params:** `date: string` — YYYY-MM-DD

**Live test:** ❌ HTTP 404 Not Found. Plausibly a device/feature gap — Training Readiness requires a compatible device/certification the test account's watch (Venu 3, a lifestyle smartwatch) may not have, rather than a bug in this tool.

### `body_battery_events`
Body battery charge/drain events for a date (meals, sleep, activity, stress).

**Params:** `date: string` — YYYY-MM-DD

**Live test:** ❌ HTTP 404 Not Found.

### `blood_pressure`
Blood pressure readings within a date range.

**Params:** `start_date: string` — YYYY-MM-DD · `end_date: string` — YYYY-MM-DD · `format: "json" | "csv"` — optional, defaults to JSON

**Live test:** ❌ HTTP 404 Not Found (test account likely has never logged a blood-pressure reading).

**Response schema (when data exists):** `EventTable` shape — JSON renders as an array of row objects; `format: "csv"` renders a header line plus one row per measurement, in the fixed column order below. Rows are extracted from whichever of Garmin's wrapper keys is present (`measurementSummaries`, `bloodPressureValues`, or `items`), or a bare top-level array.

Fixed columns:

| Field | Type | Meaning |
|---|---|---|
| measurementTimestampLocal | string | Reading timestamp, local time |
| measurementTimestampGMT | string | Reading timestamp, GMT |
| systolic | number | Systolic pressure, mmHg |
| diastolic | number | Diastolic pressure, mmHg |
| pulse | number | Pulse rate at time of reading, bpm |
| notes | string | Free-text note attached to the reading, if any |

### `floors`
Floors climbed data for a date.

**Params:** `date: string` — YYYY-MM-DD

**Live test:** ✅ Working (largest dump in this domain)

**Response schema:** Raw, un-curated Garmin JSON (`floorsChartData/daily` endpoint) — a day-window object wrapping a 15-minute-interval timeseries.

| Field | Type | Meaning |
|---|---|---|
| startTimestampGMT | string | Start of the reporting window, GMT |
| endTimestampGMT | string | End of the reporting window, GMT |
| startTimestampLocal | string | Start of the reporting window, local time |
| endTimestampLocal | string | End of the reporting window, local time |
| floorsValueDescriptorDTOList | array | Column-index legend for the tuples in `floorValuesArray` (each entry has `index: number` and `key: string`, e.g. index 2 = `"floorsAscended"`) |
| floorValuesArray | array | Array of 15-minute-interval tuples, each `[startTimeGMT: string, endTimeGMT: string, floorsAscended: number, floorsDescended: number]` per the descriptor list above |

### `rhr_day`
Resting heart rate for a specific day.

**Params:** `date: string` — YYYY-MM-DD

**Live test:** ✅ Working

**Response schema:** Raw, un-curated Garmin JSON (`userstats-service/wellness/daily` with `metricId=60`) — a metric-map wrapper rather than a flat object.

| Field | Type | Meaning |
|---|---|---|
| statisticsStartDate | string | Start of the requested range (YYYY-MM-DD) |
| statisticsEndDate | string | End of the requested range (YYYY-MM-DD) |
| userProfileId | number | Garmin internal profile identifier for the account |
| groupedMetrics | null | Reserved by Garmin's API; unpopulated for this metric |
| allMetrics.metricsMap | object | Keyed by metric name; this endpoint's request pins it to a single key |
| allMetrics.metricsMap.WELLNESS_RESTING_HEART_RATE | array | One entry per day in range, each `{ calendarDate: string, value: number }` where `value` is resting HR in bpm |

### `hydration_data`
Daily hydration data (fluid intake) for a date.

**Params:** `date: string` — YYYY-MM-DD

**Live test:** ✅ Working

**Response schema:** Raw, un-curated Garmin JSON (`usersummary-service/usersummary/hydration/daily`).

| Field | Type | Meaning |
|---|---|---|
| calendarDate | string | Calendar date (YYYY-MM-DD) |
| userId | number | Garmin internal user identifier for the account |
| valueInML | number \| null | Fluid intake logged for the day, milliliters; `null` if nothing was logged |
| goalInML | number | Daily hydration goal, milliliters |
| dailyAverageinML | number \| null | Rolling daily average intake, milliliters |
| activityIntakeInML | number \| null | Fluid intake attributed to activity/exercise, milliliters |
| sweatLossInML | number \| null | Estimated fluid loss from sweat, milliliters |
| lastEntryTimestampLocal | string \| null | Timestamp of the most recent manual log entry, local time |

### `respiration_data`
Respiration (breathing rate) summary for a date: lowest/highest/avg waking/avg sleep BPM.

**Params:** `date: string` — YYYY-MM-DD · `format: "json" | "csv"` — optional, defaults to JSON

**Live test:** ✅ Working

**Response schema:** `FlatSummary` shape (also accepts `format: "csv"`, single header + value row). Curated from Garmin's `daily/respiration` endpoint.

| Field | Type | Meaning |
|---|---|---|
| date | string | Calendar date (YYYY-MM-DD) |
| lowest_breaths_per_min | number | Lowest breathing rate recorded that day |
| highest_breaths_per_min | number | Highest breathing rate recorded that day |
| avg_waking_breaths_per_min | number | Average breathing rate while awake |
| avg_sleep_breaths_per_min | number | Average breathing rate while asleep (omitted if no sleep was recorded — not present in this live run) |

### `spo2_data`
SpO2 (blood oxygen saturation) summary for a date: avg/lowest/latest SpO2, 7-day avg, sleep SpO2.

**Params:** `date: string` — YYYY-MM-DD · `format: "json" | "csv"` — optional, defaults to JSON

**Live test:** ⚠️ No data — "No SpO2 data for 2026-08-24 — requires overnight SpO2 monitoring enabled on the watch." (Fires when the curated field map yields ≤1 populated field.)

### `all_day_events`
All-day wellness events (activity detections, move alerts, etc.) for a date.

**Params:** `date: string` — YYYY-MM-DD

**Live test:** ❌ HTTP 403 Forbidden — note this is **403**, not the 404 every other error in this domain returns. A 403 signals a permission/entitlement gate on this endpoint (the API recognizes the resource but refuses the caller), distinct from the other tools' 404s, which read as "this account has no data/feature here." Worth flagging as a different failure class rather than assuming the same "device lacks the feature" explanation.

### `hrv_data`
HRV (Heart Rate Variability) summary and status for a date.

**Params:** `date: string` — YYYY-MM-DD · `format: "json" | "csv"` — optional, defaults to JSON

**Live test:** ⚠️ No data — "No HRV data for 2026-08-24 — HRV tracking requires sleeping with an HRV-capable watch." (Tool treats a null body, a detected Garmin error envelope, or a response missing `hrvSummary` all as "no data".)

**Response schema (when data exists):** `HrvPayload` shape. JSON renders `date` + the flattened `hrvSummary` object fields + `readings_count` + `first_reading_gmt`/`last_reading_gmt` (from the first/last reading's `readingTimeGMT`) — the full per-reading array is omitted from JSON to stay compact. `format: "csv"` instead emits one row per 5-minute reading as `reading_time_gmt,hrv_value`.

### `fitnessage_data`
Fitness age data and contributing factors for a date.

**Params:** `date: string` — YYYY-MM-DD

**Live test:** ❌ HTTP 404 Not Found.

### `endurance_score`
Endurance score and detail for a date.

**Params:** `date: string` — YYYY-MM-DD

**Live test:** ❌ HTTP 404 Not Found. Plausibly a device/feature gap — Endurance Score requires a compatible/certified device the test account's watch (Venu 3, a lifestyle smartwatch) may not have, rather than a bug in this tool.

### `hill_score`
Hill score (climbing ability metric) for a date.

**Params:** `date: string` — YYYY-MM-DD

**Live test:** ❌ HTTP 404 Not Found. Plausibly a device/feature gap, same caveat as `endurance_score`.

### `lactate_threshold`
Lactate threshold data for a date.

**Params:** `date: string` — YYYY-MM-DD

**Live test:** ❌ HTTP 404 Not Found. Plausibly a device/feature gap, same caveat as `endurance_score`.

### `daily_weigh_ins`
Weigh-in records within a date range.

**Params:** `start_date: string` — YYYY-MM-DD · `end_date: string` — YYYY-MM-DD · `format: "json" | "csv"` — optional, defaults to JSON

**Live test:** ✅ Working, but the response body was `[]` — an empty array (2 bytes on disk). This test account has no weigh-ins logged for the requested range; not an error.

**Response schema:** `EventTable` shape — JSON renders as an array of row objects (empty here); `format: "csv"` renders a header line plus one row per weigh-in, in the fixed column order below. Rows are extracted from whichever of Garmin's wrapper keys is present (`dateWeightList`, `weighIns`, or `items`), or a bare top-level array.

Fixed columns:

| Field | Type | Meaning |
|---|---|---|
| calendarDate | string | Date of the weigh-in (YYYY-MM-DD) |
| weighInTimestampGMT | string | Weigh-in timestamp, GMT |
| weight | number | Body weight, grams (Garmin's raw unit) |
| bmi | number | Body mass index |
| bodyFat | number | Body fat percentage |
| bodyWater | number | Body water percentage |
| boneMass | number | Bone mass, grams |
| muscleMass | number | Muscle mass, grams |
| metabolicAge | number | Estimated metabolic age, years |
| physiqueRating | number | Garmin's physique rating score |
| visceralFat | number | Visceral fat rating |
| sourceType | string | Origin of the reading, e.g. `"manual"` or the name of a connected smart scale |

---

## Nutrition, Training Stats, User Profile, Women's Health & Data Entry

### Nutrition

### `get_nutrition_daily_food_log`
Get the daily food/nutrition log for a specific date.

**Params:** `date: string` — date to query, `YYYY-MM-DD`

**Live test:** ❌ HTTP 404 Not Found — `nutrition-service/nutrition/foodLog` returned a raw 404. Most likely explanation: this test account has never used Garmin's food-logging feature (nutrition tracking is opt-in and tied to the Garmin Connect diet/calorie tools).

### `get_nutrition_daily_settings`
Get nutrition goal settings (calorie/macro targets).

**Params:** none

**Live test:** ❌ HTTP 404 Not Found — `nutrition-service/nutrition/settings` 404. Consistent with nutrition tracking never having been enabled on this account.

### `get_custom_foods`
Get the user's custom food database.

**Params:** none

**Live test:** ❌ HTTP 404 Not Found — `nutrition-service/nutrition/foods/user` 404, same likely cause as the two tools above.

### Training Stats

### `training_status`
Get training status for a date: VO2 max, load focus, training status phrase, and acute/chronic load.

**Params:** `date: string` — date to query, `YYYY-MM-DD`

**Live test:** ✅ Working — `metrics-service/metrics/trainingstatus/aggregated/{date}` returned 200. However every metric field in the response was `null` for the queried date on this account, so the nested shape of those metrics (which the tool's description implies exists) can't be documented from this sample — likely because Garmin hadn't computed a training status for this date/account (needs a recent history of tracked activities/HRV).

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| `heatAltitudeAcclimationDTO` | null / object | Heat and altitude acclimation status; observed `null` (not computed for this date) |
| `mostRecentTrainingLoadBalance` | null / object | Anaerobic/aerobic training load balance; observed `null` |
| `mostRecentTrainingStatus` | null / object | Latest training status classification (e.g. "productive", "peaking", "maintaining") plus acute/chronic load ratio; observed `null` |
| `mostRecentVO2Max` | null / object | Latest VO2 max estimate (running and/or cycling); observed `null` |
| `userId` | number | Garmin's internal numeric account ID |

### `progress_summary`
Get weekly progress summary between two dates, optionally filtered by activity type.

**Params:** `start_date: string` — range start, `YYYY-MM-DD` · `end_date: string` — range end, `YYYY-MM-DD` · `activity_type: string` (optional) — filter, e.g. `"running"`, `"cycling"`

**Live test:** ❌ HTTP 404 Not Found — `fitnessstats-service/statistics/activities` 404. Most likely this account lacks enough tracked activity history for the fitness-stats service to have anything to aggregate.

### `race_predictions`
Get race time predictions (5K, 10K, half-marathon, marathon) based on recent training.

**Params:** none (besides client) — internally requires a resolved display name (`api.require_display_name()`) to build the request path

**Live test:** ❌ HTTP 404 Not Found — `fitnessstats-service/statistics/racePredictions/{display_name}` 404. Race predictions require sufficient recent running history to compute; plausibly absent on this account rather than a broken path (the display name itself did resolve, since the call reached Garmin with a name substituted into the URL).

### User Profile

> **Fixed (this pass):** all four `user_profile.rs` tools below were calling stale Garmin endpoints
> that 404 today — not genuine account gating. `src/auth.rs`'s own `resolve_display_name()` already
> proved a working alternative exists (`/userprofile-service/socialProfile`, 200) while
> `/userprofile-service/userprofile/v2/information` and `/userprofile-service/userprofile` both 404.
> `get_userprofile_settings` called a fourth, never-probed path
> (`/userprofile-service/social-profile/settings`) that also 404s; a working settings endpoint
> (`/userprofile-service/userprofile/settings`) was found separately. All four tools now use one of
> these two working endpoints and were re-verified live. See `src/tools/user_profile.rs` for the
> endpoint constants and rationale comment.

### `get_user_profile`
Get the user's full Garmin/social profile information (identity, privacy/visibility settings, account roles).

**Params:** none

**Live test:** ✅ Working — `GET /userprofile-service/socialProfile`

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| displayName | string | Garmin display handle (opaque identifier used as `{display_name}` in many other endpoints — not necessarily human-readable) |
| fullName / userProfileFullName | string | Account holder's full name (duplicated under two keys) |
| userName | string | **The account's login email address** — treat as sensitive |
| profileId / id | number | Internal numeric profile identifiers |
| garminGUID | string | Account-level GUID, distinct from `displayName` |
| bio, location, motivation, personalWebsite, facebookUrl, twitterUrl | string \| null | Optional public-profile fields |
| profileVisibility | string | Overall profile privacy, e.g. `"private"` |
| activityHeartRateVisibility / activityMapVisibility / activityPowerVisibility / activityStartVisibility / badgeVisibility / courseVisibility | string | Per-category sharing visibility, e.g. `"public"`, `"followers"` |
| showActivityClass / showAge / showAgeRange / showBadges / showGender / showHeight / showLast12Months / showLifetimeTotals / showPersonalRecords / showRecentDevice / showRecentFavorites / showRecentGear / showUpcomingEvents / showVO2Max / showWeight / showWeightClass | boolean | Per-field profile-display toggles |
| profileImageType | string | e.g. `"UPLOADED_PHOTO"` |
| profileImageUrlLarge / Medium / Small | string | URLs to the profile photo at each size |
| favoriteActivityTypes / favoriteCyclingActivityTypes | array | User-starred activity type keys |
| primaryActivity / otherPrimaryActivity / otherActivity / otherMotivation | string \| null | Free-form/selected activity preferences |
| runningTrainingSpeed / cyclingTrainingSpeed / swimmingTrainingSpeed / cyclingMaxAvgPower | number | Training pace/power baselines used for predictions |
| cyclingClassification | string \| null | Rider classification, if set |
| allowGolfLiveScoring / allowGolfScoringByConnections / makeGolfScorecardsPrivate | boolean | Golf-scoring sharing preferences |
| hasPremiumSocialIcon / userPro | boolean | Premium/Pro subscription indicators |
| userLevel / userPoint / userPointOffset / levelPointThreshold / levelIsViewed / levelUpdateDate | number/boolean/string | Garmin's gamification "level" system state |
| nameApproved | boolean | Whether the display name passed Garmin's moderation |
| userRoles | array of string | OAuth2 scope/role names granted to the current session's token (e.g. `SCOPE_CONNECT_READ`, `ROLE_FITNESS_USER`) |

### `get_userprofile_settings`
Get the user's locale/unit/format preferences (date, time, number, unit system, hydration container, etc.).

**Params:** none

**Live test:** ✅ Working — `GET /userprofile-service/userprofile/settings`

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| displayName | string | Garmin display handle |
| measurementSystem | string | `"metric"` or `"statute"` |
| numberFormat | string | e.g. `"decimal_period"` |
| preferredLocale | string | e.g. `"en"` |
| timeZone | string | IANA time zone name |
| hydrationMeasurementUnit | string | e.g. `"cup"` |
| hydrationContainers | array | User-defined hydration container presets (empty if none configured) |
| golfDistanceUnit / golfElevationUnit / golfSpeedUnit | string \| null | Golf-specific unit preferences |
| availableTrainingDays / preferredLongTrainingDays | array \| null | Weekday preferences for training scheduling |
| firstDayOfWeek | object | `{ dayId: number, dayName: string, isPossibleFirstDay: boolean, sortOrder: number }` |
| dateFormat / timeFormat / heartRateFormat / powerFormat | object | Each: `{ displayFormat: string \| null, formatId: number, formatKey: string, groupingUsed: boolean, maxFraction: number, minFraction: number }` — Garmin's generic formatting-preference shape, reused across several unit types |

### `get_full_name`
Get the user's full name and display name.

**Params:** none

**Live test:** ✅ Working — `GET /userprofile-service/socialProfile`, now reading `fullName` directly (previously concatenated a `firstName`/`lastName` pair that no longer exists on any working endpoint) and `displayName`.

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| full_name | string | From `socialProfile.fullName` |
| display_name | string | From `socialProfile.displayName` |

### `get_unit_system`
Get the user's preferred measurement system (metric or statute/imperial).

**Params:** none

**Live test:** ✅ Working — `GET /userprofile-service/userprofile/settings`, reading `measurementSystem`.

**Response schema:**
| Field | Type | Meaning |
|---|---|---|
| measurement_system | string | `"metric"` or `"statute"` |

### Women's Health

### `get_menstrual_data_for_date`
Get menstrual cycle data for a specific date.

**Params:** `date: string` — date to query, `YYYY-MM-DD`

**Live test:** ❌ HTTP 404 Not Found — `menstrual-service/menstrual/dayview/{date}` returned a 404 with an HTML "Page Not Found" body (not Garmin's usual JSON error envelope), consistent with menstrual tracking never having been enabled on this account rather than a malformed request.

### `get_menstrual_calendar_data`
Get menstrual calendar history within a date range.

**Params:** `start_date: string` — range start, `YYYY-MM-DD` · `end_date: string` — range end, `YYYY-MM-DD`

**Live test:** ❌ HTTP 404 Not Found — `menstrual-service/menstrual/calendar` 404, same HTML error page and likely cause as above.

### `get_pregnancy_summary`
Get pregnancy tracking summary and milestones.

**Params:** none

**Live test:** ❌ HTTP 404 Not Found — `pregnancy-service/pregnancy/snapshot` 404 with an HTML error page. Consistent with pregnancy tracking not being active on this account.

### Data Entry (write tools)

### `add_hydration_data`
Log hydration data for a date. `value_in_ml` is fluid intake in milliliters.

**Params:** `date: string` — `YYYY-MM-DD` · `value_in_ml: number` — fluid intake in milliliters

**Live test:** ⏭️ Not tested (write tool — would insert a fake hydration reading into the real account)

**Request body** (POST `usersummary-service/usersummary/hydration/daily`):
| Field | Type | Meaning |
|---|---|---|
| `calendarDate` | string | Date the reading applies to, `YYYY-MM-DD` |
| `valueInML` | number | Fluid intake in milliliters |

### `set_blood_pressure`
Record a blood pressure measurement. `date` can be `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS`.

**Params:** `date: string` — measurement date/time · `systolic: number` — systolic pressure, mmHg · `diastolic: number` — diastolic pressure, mmHg · `pulse: number` — pulse, bpm · `notes: string` (optional) — free-text note

**Live test:** ⏭️ Not tested (write tool — would insert a fake BP reading into a real health record)

**Request body** (POST `biometric-service/blood-pressure`):
| Field | Type | Meaning |
|---|---|---|
| `measurementTimestampLocal` | string | Local date/time of the reading |
| `systolic` | number | Systolic pressure (mmHg) |
| `diastolic` | number | Diastolic pressure (mmHg) |
| `pulse` | number | Pulse rate (bpm) |
| `notes` | string (optional) | Present only if supplied |

### `add_body_composition`
Record body composition data (weight required; body fat %, muscle mass, bone mass optional). Weight in kg; masses in grams.

**Params:** `date: string` — `YYYY-MM-DD` · `weight_kg: number` — body weight in kilograms (converted to grams before sending) · `percent_fat: number` (optional) — body fat percentage · `muscle_mass_grams: number` (optional) — muscle mass in grams · `bone_mass_grams: number` (optional) — bone mass in grams

**Live test:** ⏭️ Not tested (write tool — would insert fake weight/body-fat data into a real health record)

**Request body** (POST `weight-service/weight`):
| Field | Type | Meaning |
|---|---|---|
| `date` | string | Date the measurement applies to |
| `weight` | number | Weight in grams (`weight_kg * 1000`, truncated to integer) |
| `percentFat` | number (optional) | Body fat percentage; present only if supplied |
| `muscleMass` | number (optional) | Muscle mass in grams; present only if supplied |
| `boneMass` | number (optional) | Bone mass in grams; present only if supplied |

---

## Research (Date-Range Aggregates)

### `daily_stats_range`
Fetches daily health stats (steps, calories, HR, stress, body battery, SpO2, respiration) for each day in a date range, fanning out one HTTP call per day.

**Params:**
- `start_date: string` — range start, `YYYY-MM-DD`
- `end_date: string` — range end, `YYYY-MM-DD` (must be ≥ `start_date`; range capped at 366 days)
- `format: string` — `"json"` (default, array of day objects) or `"csv"`

**Live test:** ✅ Working — 10-day range returned a mix of full rows (device-worn days) and sparse rows (days with only sedentary/calorie defaults, no steps/HR/SpO2).

**Response schema:** array of 1 object per day in range, each:
| Field | Type | Meaning |
|---|---|---|
| `date` | string | Calendar date, `YYYY-MM-DD` — always present |
| `total_steps` | number | Total step count for the day |
| `total_calories` | number | Total calories burned (BMR + active), kcal |
| `active_calories` | number | Calories burned above resting baseline, kcal |
| `distance_meters` | number | Total distance traveled, meters |
| `highly_active_seconds` | number | Seconds spent in the "highly active" intensity band |
| `active_seconds` | number | Seconds spent in any active intensity band |
| `sedentary_seconds` | number | Seconds spent sedentary |
| `moderate_intensity_minutes` | number | Minutes at moderate exercise intensity |
| `vigorous_intensity_minutes` | number | Minutes at vigorous exercise intensity |
| `resting_heart_rate_bpm` | number | Resting heart rate, bpm |
| `avg_stress_level` | number | Average all-day stress score, 0–100 (`-1` = not enough data to compute) |
| `max_stress_level` | number | Peak stress score for the day |
| `body_battery_charged` | number | Body Battery points gained during the day |
| `body_battery_drained` | number | Body Battery points spent during the day |
| `body_battery_highest` | number | Highest Body Battery value reached, 0–100 |
| `body_battery_lowest` | number | Lowest Body Battery value reached, 0–100 |
| `avg_spo2_percent` | number | Average blood-oxygen saturation, % |
| `lowest_spo2_percent` | number | Lowest blood-oxygen saturation reading, % |
| `avg_waking_respiration` | number | Average respiration rate while awake, breaths/min |

Only fields Garmin actually returned that day are inserted — a day with no wearable data present just has `date` plus whatever partial fields (e.g. calorie/sedentary defaults) Garmin still supplies; there is no `null` filler. Per the tool description: *"Days without data appear as date-only rows."* In `format: "csv"` mode, absent fields render as empty cells rather than omitted keys, using the fixed 20-column header (`date, total_steps, total_calories, active_calories, distance_meters, highly_active_seconds, active_seconds, sedentary_seconds, moderate_intensity_minutes, vigorous_intensity_minutes, resting_heart_rate_bpm, avg_stress_level, max_stress_level, body_battery_charged, body_battery_drained, body_battery_highest, body_battery_lowest, avg_spo2_percent, lowest_spo2_percent, avg_waking_respiration`).

### `sleep_range`
Fetches sleep summary (total/deep/light/REM/awake time, SpO2, respiration, sleep stress, awake/restless counts) for each day in a date range.

**Params:**
- `start_date: string` — range start, `YYYY-MM-DD`
- `end_date: string` — range end, `YYYY-MM-DD` (max 366-day span)
- `format: string` — `"json"` (default) or `"csv"`

**Live test:** ✅ Working — but this account's `sleep_summary` had no underlying data for the tested period, so every row came back as `{"date": ...}` only, matching the expected date-only-row behavior.

**Response schema:** array of 1 object per day in range, each:
| Field | Type | Meaning |
|---|---|---|
| `date` | string | Calendar date, `YYYY-MM-DD` — always present |
| `total_sleep_seconds` | number | Total sleep duration, seconds |
| `deep_sleep_seconds` | number | Deep-sleep stage duration, seconds |
| `light_sleep_seconds` | number | Light-sleep stage duration, seconds |
| `rem_sleep_seconds` | number | REM-sleep stage duration, seconds |
| `awake_seconds` | number | Time awake within the sleep window, seconds |
| `sleep_start_local` | number | Sleep start timestamp, local-time epoch millis |
| `sleep_end_local` | number | Sleep end timestamp, local-time epoch millis |
| `avg_spo2_percent` | number | Average blood-oxygen saturation during sleep, % |
| `lowest_spo2_percent` | number | Lowest SpO2 reading during sleep, % |
| `avg_respiration` | number | Average respiration rate during sleep, breaths/min |
| `lowest_respiration` | number | Lowest respiration rate during sleep |
| `highest_respiration` | number | Highest respiration rate during sleep |
| `awake_count` | number | Number of distinct wake events |
| `restless_moments_count` | number | Count of detected restless-movement events |
| `avg_sleep_stress` | number | Average stress score during the sleep window |

Same absent-field behavior as `daily_stats_range`: fields are inserted only when Garmin's `dailySleepDTO` (or the top-level body, as a fallback) actually contains them; a day with no sleep record on file is just a `date`-only row rather than being dropped from the array. `format: "csv"` uses a fixed 16-column header (`date, total_sleep_seconds, deep_sleep_seconds, light_sleep_seconds, rem_sleep_seconds, awake_seconds, sleep_start_local, sleep_end_local, avg_spo2_percent, lowest_spo2_percent, avg_respiration, lowest_respiration, highest_respiration, awake_count, restless_moments_count, avg_sleep_stress`) with empty cells for missing values.

### `hrv_range`
Fetches HRV (heart-rate variability) summary for each day in a date range.

**Params:**
- `start_date: string` — range start, `YYYY-MM-DD`
- `end_date: string` — range end, `YYYY-MM-DD` (max 366-day span)
- `format: string` — `"json"` (default) or `"csv"`

**Live test:** ✅ Working — this account's `hrv_data` had no underlying data for the tested period, so every row came back as `{"date": ...}` only.

**Response schema:** array of 1 object per day in range, each:
| Field | Type | Meaning |
|---|---|---|
| `date` | string | Calendar date, `YYYY-MM-DD` — always present |
| `weekly_avg` | number | 7-day rolling average HRV, ms |
| `last_night` | number | Average overnight HRV for this night, ms |
| `last_night_5min_high` | number | Highest 5-minute HRV reading overnight, ms |
| `last_night_5min_low` | number | Lowest 5-minute HRV reading overnight, ms |
| `baseline_balanced_low` | number | Lower bound of the user's "balanced" HRV baseline range, ms |
| `baseline_balanced_upper` | number | Upper bound of the user's "balanced" HRV baseline range, ms |
| `status` | string | HRV status classification (e.g. `"BALANCED"`, `"UNBALANCED"`, `"LOW"`) |
| `feedback` | string | Garmin's textual/coded feedback string for the night |

Per the source doc comment: *"Days without HRV data appear as date-only rows."* — same insert-only-if-present pattern as the other range tools. `format: "csv"` uses a fixed 9-column header (`date, weekly_avg, last_night, last_night_5min_high, last_night_5min_low, baseline_balanced_low, baseline_balanced_upper, status, feedback`) with empty cells for missing values.

### `weekly_summary`
Computes per-ISO-week statistics (mean, std, min, max) over 12 key daily-health-stat metrics across a date range, by fetching the same per-day data as `daily_stats_range` and grouping/aggregating it by ISO week.

**Params:**
- `start_date: string` — range start, `YYYY-MM-DD`
- `end_date: string` — range end, `YYYY-MM-DD` (max 366-day span)

Note: unlike the other three range tools, `weekly_summary` takes **no `format` param** — it always returns JSON (no CSV mode exists in source for this tool).

**Live test:** ✅ Working — an 11-day range spanning parts of 3 ISO weeks returned 3 week objects with correctly bucketed `days_with_data` counts and computed stats.

**Response schema:** array of 1 object per ISO week touched by the range, each:
| Field | Type | Meaning |
|---|---|---|
| `week` | string | ISO week label, `<year>-W<2-digit week>` (e.g. `"2026-W34"`) |
| `days_with_data` | number | Count of days in this week (within the requested range) that had at least a fetched row |
| `week_start` | string | Date of the first day of this week present in the range, `YYYY-MM-DD` |
| `week_end` | string | Date of the last day of this week present in the range, `YYYY-MM-DD` |
| `<metric>_mean` | number | Arithmetic mean of `<metric>` across days in the week that had a value |
| `<metric>_std` | number | Population standard deviation of `<metric>` across those days |
| `<metric>_min` | number | Minimum observed value of `<metric>` in the week |
| `<metric>_max` | number | Maximum observed value of `<metric>` in the week |

The `<metric>_mean/_std/_min/_max` quartet is emitted independently for each of 12 numeric metrics: `total_steps`, `total_calories`, `active_calories`, `distance_meters`, `moderate_intensity_minutes`, `vigorous_intensity_minutes`, `resting_heart_rate_bpm`, `avg_stress_level`, `body_battery_highest`, `body_battery_lowest`, `avg_spo2_percent`, `avg_waking_respiration` — so a full week object has up to 4 (fixed fields) + 12×4 = 52 keys. A metric's quartet is omitted entirely for a week if no day in that week had a value for it. All stat values are rounded to 2 decimal places.

---

## Workouts

### `get_workouts`
List workouts saved to the account's workout library.

**Params:** `start: u32` — pagination offset; `limit: u32` — max results to return (clamped 1–100)

**Live test:** ❌ HTTP 405 Method Not Allowed — `workout-service/workout` (Garmin error body: `NotAllowedException`). Distinct from the 404s seen elsewhere in this dump run: a 405 means the endpoint exists but rejected this request's verb/shape, not that the account has never used the feature. Possible causes: Garmin now expects a different HTTP method (e.g. POST with a search body) or different query parameters than the `start`/`limit` pair this tool sends. Worth re-checking against current Garmin Connect web traffic before assuming this tool still works as written.

### `get_workout_by_id`
Fetch a single saved workout's full definition by ID.

**Params:** `workout_id: &str` — the workout's Garmin-assigned ID

**Live test:** ⏭️ Not tested — this test account has zero resolvable workouts (see `get_workouts` above), so no `workout_id` was available to exercise. Request shape only: `GET /workout-service/workout/{workout_id}`, no query params. Response, if any, would be Garmin's structured workout definition (steps, targets, sport type) — schema not observed.

### `get_scheduled_workouts`
List workouts scheduled onto the calendar within a date range.

**Params:** `start_date: &str` — range start, `YYYY-MM-DD`; `end_date: &str` — range end, `YYYY-MM-DD`

**Live test:** ❌ HTTP 404 Not Found — `workout-service/schedule`. Most likely this test account has no workout schedule feature/data in use rather than a broken endpoint (consistent with the other 404s in this dump run), though a genuinely renamed/removed endpoint can't be ruled out from this signal alone.

### `delete_workout`
Delete a saved workout by ID.

**Params:** `workout_id: &str` — the workout's Garmin-assigned ID

**Live test:** ⏭️ Not tested (destructive write tool, deliberately not exercised). Request shape only: `DELETE /workout-service/workout/{workout_id}`, no body. Returns a plain success/error string, not JSON — no response schema to document.

### `schedule_workout`
Schedule an existing saved workout onto a specific calendar date.

**Params:** `workout_id: &str` — the workout's Garmin-assigned ID; `date: &str` — calendar date to schedule onto, `YYYY-MM-DD`

**Live test:** ⏭️ Not tested (write tool, deliberately not exercised — would add a real calendar entry). Request shape only: `POST /workout-service/schedule/{workout_id}` with JSON body `{"date": "<date>"}`. Response is Garmin's raw JSON echoed back pretty-printed (unlike other tools in this domain, it does not pass through `render_or_friendly`) — schema not observed since untested.
