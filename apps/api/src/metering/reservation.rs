use sqlx::{PgExecutor, PgPool};

/// Token counts for a settlement. A `None` count is written as 0, never NULL.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub cached_tokens: Option<i32>,
    /// Locally counted rather than reported by the provider.
    pub estimated: bool,
}

/// Settles reservation `id`. Always overwrites, so a late owner's real usage replaces a
/// sweeper's estimate; repeating it is harmless.
pub async fn settle<'e>(db: impl PgExecutor<'e>, id: i64, usage: Usage) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE llm_usage SET prompt_tokens = coalesce($2, 0), completion_tokens = coalesce($3, 0), \
         cached_tokens = coalesce($4, 0), estimated = $5, settled_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(usage.prompt_tokens)
    .bind(usage.completion_tokens)
    .bind(usage.cached_tokens)
    .bind(usage.estimated)
    .execute(db)
    .await
    .map(drop)
}

/// Guard for one admitted LLM call. Dropping it unsettled (error path, cancellation, panic)
/// spawns a settle from the in-memory counts with `estimated = true`.
#[must_use = "an unsettled reservation is settled as estimated on drop"]
pub struct Reservation {
    id: i64,
    pool: PgPool,
    counted: Usage,
    settled: bool,
}

impl Reservation {
    pub(super) fn new(id: i64, pool: PgPool) -> Self {
        Reservation {
            id,
            pool,
            counted: Usage::default(),
            settled: false,
        }
    }

    pub fn id(&self) -> i64 {
        self.id
    }

    /// Records the locally counted prompt and streamed completion so far.
    pub fn count(&mut self, prompt_tokens: i32, completion_tokens: i32) {
        self.counted.prompt_tokens = Some(prompt_tokens);
        self.counted.completion_tokens = Some(completion_tokens);
    }

    /// Settles with the given usage. If the write fails, the drop fallback still runs.
    pub async fn settle(mut self, usage: Usage) -> sqlx::Result<()> {
        settle(&self.pool, self.id, usage).await?;
        self.settled = true;
        Ok(())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        // ponytail: no runtime (process teardown) → left to the sweeper's 15-min safety net
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (pool, id) = (self.pool.clone(), self.id);
        let usage = Usage {
            estimated: true,
            ..self.counted
        };
        rt.spawn(async move {
            if let Err(e) = settle(&pool, id, usage).await {
                eprintln!("reservation {id}: drop settle failed: {e}");
            }
        });
    }
}
