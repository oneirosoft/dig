pub(crate) mod adopt;
pub(crate) mod branch;
pub(crate) mod clean;
pub(crate) mod commit;
pub(crate) mod deleted_local;
pub(crate) mod git;
pub(crate) mod graph;
pub(crate) mod init;
pub(crate) mod merge;
pub(crate) mod orphan;
pub(crate) mod reparent;
pub(crate) mod restack;
pub(crate) mod store;
pub(crate) mod sync;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod tree;
pub(crate) mod workflow;

#[cfg(test)]
pub(crate) fn test_cwd_lock() -> &'static std::sync::Mutex<()> {
    use std::sync::{Mutex, OnceLock};

    static CWD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    CWD_LOCK.get_or_init(|| Mutex::new(()))
}
