//! Named async locks (`App::suggestion_locks`): operations on the same key serialize, others run in parallel.
//! Suggestions use one per suggestion, so GitHub never holds the pre-draft gate; Home Git, the repository
//! cache and the vault use their own keys. Unused locks are dropped.

type LockMap = std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>;
#[derive(Clone, Default)]
pub struct Locks(std::sync::Arc<std::sync::Mutex<LockMap>>);
impl Locks {
    pub fn get(&self, id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.0.lock().unwrap();
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(id).and_then(std::sync::Weak::upgrade) { return lock; }
        let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(id.to_string(), std::sync::Arc::downgrade(&lock));
        lock
    }
}
