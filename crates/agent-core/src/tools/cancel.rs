use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

/// One token cancels a whole task (plan 015, D6): the loop checks it between steps,
/// the model stream aborts on it, and a running command has its process tree killed.
///
/// Cloning shares the same state, so every holder sees the same cancellation.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<Inner>);

#[derive(Debug, Default)]
struct Inner {
    flag: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn cancel(&self) {
        self.0.flag.store(true, Ordering::SeqCst);
        self.0.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(Ordering::SeqCst)
    }

    /// Resolves once cancelled, immediately if it already was.
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            let notified = self.0.notify.notified();
            tokio::pin!(notified);
            // Register before re-reading the flag, so a cancel in between still wakes us.
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn clones_share_the_same_state() {
        let token = CancelToken::default();
        let clone = token.clone();
        assert!(!token.is_cancelled());
        clone.cancel();
        assert!(token.is_cancelled());
        assert!(clone.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_future_resolves() {
        let already = CancelToken::default();
        already.cancel();
        tokio::time::timeout(Duration::from_secs(1), already.cancelled())
            .await
            .expect("an already cancelled token resolves right away");

        let token = CancelToken::default();
        let waiter = token.clone();
        let handle = tokio::spawn(async move { waiter.cancelled().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("cancel() wakes a pending cancelled()")
            .unwrap();
    }
}
