// SPDX-License-Identifier: Apache-2.0

//! Small helpers shared by the async code.

use std::{
    future::Future,
    pin::Pin,
    sync::{Mutex, MutexGuard, PoisonError},
};

use tokio::task::JoinSet;

use crate::error::{Error, Result};

/// A boxed future, for traits that must stay usable as `dyn`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Locks `mutex`, carrying on if another thread panicked while holding it.
///
/// Everything guarded this way is bookkeeping that is updated in one step, so a
/// panic cannot leave it half-changed; refusing to continue would turn one bug
/// into a permanent failure.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs file work on a thread meant for blocking, so it cannot stall the network tasks.
///
/// A panic inside `work` is passed on to the caller. If the runtime is shutting
/// down and the work never ran, this reports [`Error::Stopped`].
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T> {
    match tokio::task::spawn_blocking(work).await {
        Ok(value) => Ok(value),
        Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
        Err(_) => Err(Error::Stopped),
    }
}

/// Runs `work` as a task of its own, so that a panic in it stays there.
///
/// The router stops accepting connections for good if any protocol handler
/// panics, which would make any bug the network can reach a remote off switch.
/// Dropping the returned future aborts the task.
pub(crate) async fn contained(what: &'static str, work: impl Future<Output = ()> + Send + 'static) {
    let mut task = JoinSet::new();
    task.spawn(work);
    if let Some(Err(e)) = task.join_next().await
        && e.is_panic()
    {
        tracing::error!(what, "a task panicked; Baton carries on without it");
    }
}

/// `N` random bytes from the operating system.
pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| Error::network("getting random numbers", &e))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn a_panic_in_contained_work_does_not_reach_the_caller() {
        contained("test", async { panic!("on purpose") }).await;
    }

    #[tokio::test]
    async fn contained_work_runs_to_completion() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        contained("test", async move {
            tx.send(7).unwrap();
        })
        .await;
        assert_eq!(rx.await.unwrap(), 7);
    }

    #[tokio::test]
    async fn dropping_contained_work_stops_it() {
        let alive = Arc::new(());
        let held = Arc::clone(&alive);
        let work = contained("test", async move {
            let held = held;
            std::future::pending::<()>().await;
            drop(held);
        });
        // Poll once so the task starts, then drop the future.
        let _ = tokio::time::timeout(std::time::Duration::from_millis(20), work).await;
        // Give the aborted task a moment to release what it held.
        for _ in 0..50 {
            if Arc::strong_count(&alive) == 1 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the task was not stopped");
    }

    #[test]
    fn a_poisoned_lock_is_still_usable() {
        let mutex = Arc::new(Mutex::new(1));
        let poisoner = Arc::clone(&mutex);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert_eq!(*lock(&mutex), 1);
    }

    #[test]
    fn random_bytes_differ() {
        assert_ne!(random_bytes::<16>().unwrap(), random_bytes::<16>().unwrap());
    }
}
