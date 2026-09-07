//! Prune placeholder (filled in a later task).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keep {
    pub daily: u32,
    pub weekly: u32,
    pub monthly: u32,
    pub yearly: u32,
}

