//! Cron scheduler (local time) + boot catch-up. Ported from the POC; timezone
//! changed from UTC to `chrono::Local`, overlap handled by `running::try_start`.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDateTime};
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};
use uuid::Uuid;

use crate::backup::{plans::{self, Plan}, run, runs};
use crate::state::AppState;

struct Handle {
    scheduler: JobScheduler,
    jobs: Mutex<HashMap<i64, Uuid>>,
}
static HANDLE: OnceLock<Handle> = OnceLock::new();

pub fn to_scheduler_expr(five_field: &str) -> String {
    format!("0 {}", five_field.trim())
}

/// Validate a 5-field cron expression without a running scheduler: building a
/// throwaway `Job` parses the expression the same way `register_plan` will.
pub fn parse_cron(five_field: &str) -> Result<(), String> {
    Job::new_async_tz(to_scheduler_expr(five_field).as_str(), Local, |_id, _l| Box::pin(async {}))
        .map(|_| ())
        .map_err(|e| format!("รูปแบบ cron ไม่ถูกต้อง: {e}"))
}

fn is_scheduled(plan: &Plan) -> bool {
    plan.active && plan.schedule_cron.as_deref().map(|c| !c.trim().is_empty()).unwrap_or(false)
}

/// Build the scheduler, register every active scheduled plan, start ticking.
pub async fn start(state: AppState) -> Result<(), JobSchedulerError> {
    let scheduler = JobScheduler::new().await?;
    if HANDLE.set(Handle { scheduler: scheduler.clone(), jobs: Mutex::new(HashMap::new()) }).is_err() {
        tracing::warn!("scheduler: already started");
        return Ok(());
    }
    match plans::list_all(&state.pool).await {
        Ok(all) => for p in all.iter().filter(|p| is_scheduled(p)) { register_plan(&state, p).await; },
        Err(e) => tracing::error!("scheduler: cannot load plans: {e}"),
    }
    scheduler.start().await
}

pub async fn register_plan(state: &AppState, plan: &Plan) {
    let Some(handle) = HANDLE.get() else { return };
    if !is_scheduled(plan) {
        return;
    }
    let expr = to_scheduler_expr(plan.schedule_cron.as_deref().unwrap_or(""));
    let (st, pl) = (state.clone(), plan.clone());
    let job = Job::new_async_tz(expr.as_str(), Local, move |_id, _l| {
        let (st, pl) = (st.clone(), pl.clone());
        Box::pin(async move {
            let out = run::run_backup(&st, &pl, "schedule").await;
            if out.status == "skipped" {
                tracing::warn!(plan = pl.id, "scheduler: previous run still going, skipped tick");
            }
        })
    });
    match job {
        Ok(job) => match handle.scheduler.add(job).await {
            Ok(id) => {
                handle.jobs.lock().await.insert(plan.id, id);
                tracing::info!(plan = plan.id, cron = expr, "scheduler: registered (local time)");
            }
            Err(e) => tracing::error!(plan = plan.id, "scheduler: add failed: {e}"),
        },
        Err(e) => tracing::warn!(plan = plan.id, cron = ?plan.schedule_cron, "scheduler: invalid cron, skipped: {e}"),
    }
}

/// Removes the job from the running scheduler, then drops its id from the map. If
/// the scheduler-side removal fails, the map entry is left in place (not lost) so a
/// later call can retry rather than orphaning a job the map no longer tracks.
pub async fn unregister_plan(id: i64) {
    let Some(handle) = HANDLE.get() else { return };
    let Some(job_id) = handle.jobs.lock().await.get(&id).copied() else { return };
    match handle.scheduler.remove(&job_id).await {
        Ok(()) => {
            handle.jobs.lock().await.remove(&id);
        }
        Err(e) => tracing::error!(plan = id, "scheduler: remove failed: {e}"),
    }
}

/// Next tick in local time, or `None` when not scheduled / scheduler not running.
pub async fn next_run_at(plan_id: i64) -> Option<DateTime<Local>> {
    let handle = HANDLE.get()?;
    let job_id = *handle.jobs.lock().await.get(&plan_id)?;
    let mut scheduler = handle.scheduler.clone();
    scheduler.next_tick_for_job(job_id).await.ok().flatten().map(|t| t.with_timezone(&Local))
}

/// No ok/partial run in the last 24 h → run soon after boot.
pub fn needs_catch_up(last_success: Option<NaiveDateTime>, now: NaiveDateTime) -> bool {
    match last_success {
        None => true,
        Some(t) => now - t > chrono::Duration::hours(24),
    }
}

/// Spec §8: for each active, scheduled, catch_up plan that missed its window, start a
/// run 120 s + 30 s·i after boot (staggered so they queue on the dump semaphore).
pub async fn catch_up(state: AppState) {
    let Ok(all) = plans::list_all(&state.pool).await else { return };
    let now = Local::now().naive_local();
    let mut i = 0u64;
    for plan in all.into_iter().filter(|p| is_scheduled(p) && p.catch_up) {
        let last = runs::last_success_at(&state.pool, plan.id).await.ok().flatten()
            .and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok());
        if !needs_catch_up(last, now) {
            continue;
        }
        let delay = Duration::from_secs(120 + 30 * i);
        i += 1;
        tracing::info!(plan = plan.id, ?delay, "catch-up scheduled");
        let st = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            // A scheduled tick may have fired during the delay — re-check before running.
            let last = runs::last_success_at(&st.pool, plan.id).await.ok().flatten()
                .and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok());
            if !needs_catch_up(last, Local::now().naive_local()) {
                tracing::info!(plan = plan.id, "catch-up: a regular tick already ran during the delay, skipping");
                return;
            }
            run::run_backup(&st, &plan, "catch_up").await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn t(d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, d).unwrap().and_hms_opt(h, 0, 0).unwrap()
    }

    #[test]
    fn expr_prepends_seconds() {
        assert_eq!(to_scheduler_expr("30 3 * * *"), "0 30 3 * * *");
        assert_eq!(to_scheduler_expr("  0 20 * * 0 "), "0 0 20 * * 0");
    }

    #[test]
    fn catch_up_rule_is_24h() {
        assert!(needs_catch_up(None, t(7, 12)));
        assert!(needs_catch_up(Some(t(5, 3)), t(7, 12)));
        assert!(!needs_catch_up(Some(t(7, 3)), t(7, 12)));
    }

    #[tokio::test]
    async fn parse_cron_accepts_valid_rejects_invalid() {
        assert!(parse_cron("30 3 * * *").is_ok());
        assert!(parse_cron("0 20 * * 0").is_ok());
        assert!(parse_cron("not a cron").is_err());
        assert!(parse_cron("99 99 * * *").is_err());
        // HANDLE is unset in unit tests → next_run_at is None regardless of cron validity.
        assert!(next_run_at(1).await.is_none());
    }
}
