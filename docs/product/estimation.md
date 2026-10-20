# Capacity estimate

_All values are computed by `factory estimate` from the inputs below. Inputs are assumptions, not measurements. Validate with `factory benchmark`._

## Inputs (assumptions)

| Input | Value | Unit | Meaning | Source/rationale |
|---|---|---|---|---|
| daily_active_users | 50.00 | users/day | distinct users per day | agent proposal: Guess. Sign-up is invite-only (AC-1.1, ADR 0004) and the phase 1 gate is tested with 5 self-learners (success metrics). 50 is a generous ceiling for year one so the estimate errs high. |
| requests_per_user_per_day | 200 | req/user/day | API/page requests per active user | agent proposal: Guess. A 20-minute study session has around 20–40 questions (J1–J4). Each question costs several calls: POST message, SSE stream, token-counter refresh (FR-12), position/collapse saves (FR-15/16, FR-19) and event logging. Add canvas list/open calls and round up. |
| peak_to_average | 10.00 | ratio | peak-hour RPS / average RPS | agent proposal: Guess. Few users who study in bursts (moderated sessions, evenings), so a high ratio is the conservative choice. |
| peak_concurrent_user_fraction | 0.2000 | ratio | share of DAU active in the same minute at peak | agent proposal: Guess. Small invite-only cohorts may run sessions at the same time, as in the moderated gate test (success metrics). 20% is high on purpose. |
| read_fraction | 0.7000 | ratio | share of requests that are reads | agent proposal: Guess. Every question writes 2 nodes, 1 edge, llm_usage rows and events (FR-3, NFR-12). Position, collapse and merge saves are writes too, so reads are a lower share than in a typical web app. |
| avg_response_kb | 8.00 | KB | average response payload | agent proposal: Guess. Most responses are small JSON or streamed answers of a few KB. Canvas opens (GET /api/canvases/:id, up to 200–1,000 nodes, NFR-1/NFR-3) are large, which pulls the average up. |
| avg_write_kb | 4.00 | KB | bytes stored per write | agent proposal: Guess. An ai_message node of about 2–4 KB of text, plus edge/anchor, llm_usage and event rows and index overhead (FR-19, NFR-12). |
| retention_days | 365 | days | how long written data is kept | agent proposal: Data is kept until the user deletes it (FR-19, FR-20), with no expiry rule. Assumes it all lasts the whole first year. Backups add 7 days (NFR-11). |
| avg_latency_ms | 2,000 | ms | average server time per request | agent proposal: Guess. Most API calls take tens of ms (NFR-1: context build p95 < 200 ms), but SSE answer streams hold the request open for the whole LLM reply (FR-3; ARCHITECTURE: streaming runs in the API request). Those dominate the average, so 2 s is used. |
| cache_hot_fraction | 0.2000 | ratio | share of stored data that is hot enough to cache | agent proposal: Guess. Users mostly reopen their recent canvases (Return metric: second day within 7 days). Older canvases are cold. |
| egress_usd_per_gb | 0.0000 | USD/GB | bandwidth price (from your provider; 0 if unknown) | agent proposal: 0 = unknown. The host is not chosen yet (Fly.io, Render or Railway are the candidates in ARCHITECTURE/review F-L3-9). |
| storage_usd_per_gb_month | 0.0000 | USD/GB-month | storage price (from your provider; 0 if unknown) | agent proposal: 0 = unknown. Same reason: the Postgres host and pricing are not chosen yet. |

## Results

| Quantity | Value | Formula |
|---|---|---|
| daily_requests | 10,000 | DAU × requests_per_user_per_day |
| avg_rps | 0.1157 | daily_requests / 86 400 s |
| peak_rps | 1.16 | avg_rps × peak_to_average |
| peak_read_rps | 0.8102 | peak_rps × read_fraction |
| peak_write_rps | 0.3472 | peak_rps × (1 − read_fraction) |
| concurrent_active_users | 10.00 | DAU × peak_concurrent_user_fraction (people, not requests) |
| concurrent_in_flight_requests | 2.31 | peak_rps × avg_latency (Little's law) |
| peak_bandwidth_mbps | 0.0723 | peak_rps × avg_response_kb × 8 / 1024 |
| writes_per_day | 3,000 | daily_requests × (1 − read_fraction) |
| storage_growth_gb_per_day | 0.0114 | writes_per_day × avg_write_kb / 1024² |
| retained_storage_gb | 4.18 | storage_growth_gb_per_day × retention_days |
| cache_size_gb | 0.8354 | retained_storage_gb × cache_hot_fraction |
| egress_gb_per_month | 2.29 | daily_requests × 30 × avg_response_kb / 1024² |
| est_monthly_egress_usd | 0.0000 | egress_gb_per_month × egress_usd_per_gb |
| est_monthly_storage_usd | 0.0000 | retained_storage_gb × storage_usd_per_gb_month |

Active users ≠ simultaneous requests: 10.00 people active at peak generate only ~2.31 requests in flight at once.

## Sensitivity

| Traffic multiplier | peak RPS | in-flight | storage GB retained | egress GB/mo |
|---|---|---|---|---|
| ×0.5 | 0.5787 | 1.16 | 2.09 | 1.14 |
| ×1 | 1.16 | 2.31 | 4.18 | 2.29 |
| ×2 | 2.31 | 4.63 | 8.35 | 4.58 |
| ×10 | 11.57 | 23.15 | 41.77 | 22.89 |

## Implication (heuristic)

Peak load is small: one application instance plus a managed database is sufficient. No cache, queue, replicas or sharding are justified by load.

Costs are only as good as the unit prices entered; 0 means unknown, not free.
