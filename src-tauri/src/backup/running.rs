//! Which plans are backing up right now (process-global, ephemeral).

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

static RUNNING: OnceLock<Mutex<HashSet<i64>>> = OnceLock::new();

fn set() -> &'static Mutex<HashSet<i64>> {
    RUNNING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Claim the plan. `false` = already running (caller must skip).
pub fn try_start(plan_id: i64) -> bool {
    set().lock().unwrap().insert(plan_id)
}

pub fn end(plan_id: i64) {
    set().lock().unwrap().remove(&plan_id);
}

pub fn is_running(plan_id: i64) -> bool {
    set().lock().unwrap().contains(&plan_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_is_exclusive_until_end() {
        assert!(try_start(9001));
        assert!(!try_start(9001));
        assert!(is_running(9001));
        end(9001);
        assert!(!is_running(9001));
        assert!(try_start(9001));
        end(9001);
    }
}
