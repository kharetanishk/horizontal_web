//! Reservation ledger over `llm_usage` (docs/ARCHITECTURE.md, Metering).

mod reservation;

pub use reservation::{Reservation, Usage, settle};

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::error::ApiError;

/// Concurrent user-initiated calls allowed per user; the next one gets 429.
pub const MAX_IN_FLIGHT: i64 = 3;

// ponytail: context constants live here until the context module (T9) owns them.
const BLOCK_TOKENS: f64 = 2_000.0;
const PATH_MSG_MAX_TOKENS: f64 = 2_500.0;
const FRAME_TOKENS: f64 = 8.0;
const COMPACTION_SUMMARY_MAX_TOKENS: f64 = 700.0;
/// 5% provider-framing margin on every reservation.
const MARGIN: f64 = 1.05;

/// Input ≤ 14,000 plus output ≤ 2,000, plus the margin.
pub const ANSWER_RESERVE: i32 = 16_800;
/// Input 4,000 plus output 100, plus the margin.
pub const MERGE_PROPOSAL_RESERVE: i32 = 4_305;

/// Bounded summary input in the summary model's tokens, plus its output, plus the margin.
pub fn compaction_reserve(summary_r_up: f64) -> i32 {
    let input = (COMPACTION_SUMMARY_MAX_TOKENS + BLOCK_TOKENS + PATH_MSG_MAX_TOKENS + FRAME_TOKENS)
        * summary_r_up;
    ((input + COMPACTION_SUMMARY_MAX_TOKENS) * MARGIN).ceil() as i32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Answer,
    Compaction,
    MergeProposal,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Answer => "answer",
            Kind::Compaction => "compaction",
            Kind::MergeProposal => "merge_proposal",
        }
    }
}

/// Admission config plus the pool that guards settle through.
#[derive(Clone)]
pub struct Meter {
    pool: PgPool,
    daily_cap: i64,
    compaction_reserve: i32,
}

impl Meter {
    /// Startup check: every reservation must be ≤ `daily_cap / 10`.
    pub fn new(pool: PgPool, daily_cap: i64, summary_r_up: f64) -> Result<Self, String> {
        let meter = Meter {
            pool,
            daily_cap,
            compaction_reserve: compaction_reserve(summary_r_up),
        };
        for kind in [Kind::Answer, Kind::Compaction, Kind::MergeProposal] {
            let size = meter.reserve(kind);
            if i64::from(size) * 10 > daily_cap {
                return Err(format!(
                    "{} reservation {size} exceeds DAILY_TOKEN_CAP / 10 ({daily_cap} / 10)",
                    kind.as_str()
                ));
            }
        }
        Ok(meter)
    }

    pub fn reserve(&self, kind: Kind) -> i32 {
        match kind {
            Kind::Answer => ANSWER_RESERVE,
            Kind::Compaction => self.compaction_reserve,
            Kind::MergeProposal => MERGE_PROPOSAL_RESERVE,
        }
    }

    /// Admits one LLM call inside the caller's transaction, locking the user row first.
    /// On error nothing was inserted; the caller rolls back.
    pub async fn admit(
        &self,
        tx: &mut PgConnection,
        user_id: Uuid,
        kind: Kind,
        model: &str,
        node_id: Option<Uuid>,
        owner_key: i64,
    ) -> Result<Reservation, ApiError> {
        // serializes metering per user without blocking FK inserts
        sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR NO KEY UPDATE")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        // the 3-arg date_trunc pins the UTC day whatever the session TimeZone
        let (spent_today, in_flight, until_midnight): (i64, i64, i64) = sqlx::query_as(
            "SELECT coalesce(sum(CASE WHEN settled_at IS NULL THEN reserved_tokens \
                     ELSE coalesce(prompt_tokens, 0) + coalesce(completion_tokens, 0) END), 0)::bigint, \
                    count(*) FILTER (WHERE settled_at IS NULL AND kind <> 'compaction'), \
                    ceil(extract(epoch FROM date_trunc('day', now(), 'UTC') + interval '1 day' - now()))::bigint \
             FROM llm_usage \
             WHERE user_id = $1 AND (created_at >= date_trunc('day', now(), 'UTC') OR settled_at IS NULL)",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;

        let reserve = self.reserve(kind);
        if spent_today + i64::from(reserve) > self.daily_cap {
            return Err(ApiError::TooManyRequests {
                code: "daily_cap_reached",
                retry_after: until_midnight.max(1) as u64,
            });
        }
        if kind != Kind::Compaction && in_flight >= MAX_IN_FLIGHT {
            return Err(ApiError::TooManyRequests {
                code: "rate_limited",
                retry_after: 1,
            });
        }

        let id: i64 = sqlx::query_scalar(
            "INSERT INTO llm_usage (user_id, node_id, kind, model, owner_key, reserved_tokens) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(user_id)
        .bind(node_id)
        .bind(kind.as_str())
        .bind(model)
        .bind(owner_key)
        .bind(reserve)
        .fetch_one(&mut *tx)
        .await?;
        Ok(Reservation::new(id, self.pool.clone()))
    }
}
