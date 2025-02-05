use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, Weak};

use futures_util::Future;
use tokio::sync::broadcast;

pub trait Key: Send + Sync + Eq + Hash + Clone {}
impl<A: Send + Sync + Eq + Hash + Clone> Key for A {}

pub trait Value: Send + Sync {}
impl<A: Send + Sync> Value for A {}

///
/// Allows deduplication of async operations based on a key.
pub struct Dedupe<Key, Value> {
    /// Cache storage for the operations.
    cache: Arc<Mutex<HashMap<Key, State<Value>>>>,
    /// Initial size of the multi-producer, multi-consumer channel.
    size: usize,
    /// When enabled allows the operations to be cached forever.
    persist: bool,
}

/// Represents the current state of the operation.
enum State<Value> {
    /// Means that the operation has been executed and the result is stored.
    Ready(Arc<Value>),

    /// Means that the operation is in progress and the result can be sent via
    /// the stored sender whenever it's available in the future.
    Pending(Weak<broadcast::Sender<Arc<Value>>>),
}

/// Represents the next steps
enum Step<Value> {
    /// The operation has been executed and the result must be returned.
    Return(Arc<Value>),

    /// The operation is in progress and the result must be awaited on the
    /// receiver.
    Await(broadcast::Receiver<Arc<Value>>),

    /// The operation needs to be executed and the result needs to be sent to
    /// the provided sender.
    Init(Arc<broadcast::Sender<Arc<Value>>>),
}

impl<K: Key, V: Value> Dedupe<K, V> {
    pub fn new(size: usize, persist: bool) -> Self {
        Self { cache: Arc::new(Mutex::new(HashMap::new())), size, persist }
    }

    pub async fn dedupe<'a, Fn, Fut>(&'a self, key: &'a K, or_else: Fn) -> Arc<V>
    where
        Fn: FnOnce() -> Fut,
        Fut: Future<Output = Arc<V>>,
    {
        loop {
            let value = match self.step(key) {
                Step::Return(value) => value,
                Step::Await(mut rx) => match rx.recv().await {
                    Ok(value) => value,
                    Err(_) => {
                        // If we get an error that means the task with
                        // owned tx (sender) was dropped.i.e. there is no result in cache
                        // and we can try another attempt because probably another
                        // task will do the execution
                        continue;
                    }
                },
                Step::Init(tx) => {
                    let value = or_else().await;
                    let mut guard = self.cache.lock().unwrap();
                    if self.persist {
                        guard.insert(key.to_owned(), State::Ready(value.clone()));
                    } else {
                        guard.remove(key);
                    }
                    let _ = tx.send(value.clone());
                    value
                }
            };

            return value;
        }
    }

    fn step(&self, key: &K) -> Step<V> {
        let mut this = self.cache.lock().unwrap();

        if let Some(state) = this.get(key) {
            match state {
                State::Ready(value) => return Step::Return(value.clone()),
                State::Pending(tx) => {
                    // We can upgrade from Weak to Arc only in case when
                    // original tx is still alive
                    // otherwise we will create in the code below
                    if let Some(tx) = tx.upgrade() {
                        return Step::Await(tx.subscribe());
                    }
                }
            }
        }

        let (tx, _) = broadcast::channel(self.size);
        let tx = Arc::new(tx);
        // Store a Weak version of tx and pass actual tx to further handling
        // to control if tx is still alive and will be able to handle the request.
        // Only single `strong` reference to tx should exist so we can
        // understand when the execution is still alive and we'll get the response
        this.insert(key.to_owned(), State::Pending(Arc::downgrade(&tx)));
        Step::Init(tx)
    }
}

pub struct DedupeResult<K, V, E>(Dedupe<K, Result<V, E>>);

impl<K: Key, V: Value, E: Value> DedupeResult<K, V, E> {
    pub fn new(persist: bool) -> Self {
        Self(Dedupe::new(1, persist))
    }
}

impl<K: Key, V: Value, E: Value> DedupeResult<K, V, E> {
    pub async fn dedupe<'a, Fn, Fut>(&'a self, key: &'a K, or_else: Fn) -> Arc<Result<V, E>>
    where
        Fn: FnOnce() -> Fut,
        Fut: Future<Output = Arc<Result<V, E>>>,
    {
        self.0.dedupe(key, or_else).await
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Deref;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use assert_eq;
    use tokio::join;
    use tokio::time::{sleep, timeout_at, Instant};

    use super::*;

    #[tokio::test]
    async fn test_no_key() {
        let cache = Arc::new(Dedupe::<u64, u64>::new(1000, true));
        let actual = cache.dedupe(&1, || Box::pin(async { Arc::new(1) })).await;
        assert_eq!(*actual, 1);
    }

    #[tokio::test]
    async fn test_with_key() {
        let cache = Arc::new(Dedupe::<u64, u64>::new(1000, true));
        cache.dedupe(&1, || Box::pin(async { Arc::new(1) })).await;

        let actual = cache.dedupe(&1, || Box::pin(async { Arc::new(2) })).await;
        assert_eq!(*actual, 1);
    }

    #[tokio::test]
    async fn test_with_multi_get() {
        let cache = Arc::new(Dedupe::<u64, u64>::new(1000, true));

        for i in 0..100 {
            cache
                .dedupe(&1, || Box::pin(async move { Arc::new(i) }))
                .await;
        }

        let actual = cache.dedupe(&1, || Box::pin(async { Arc::new(2) })).await;
        assert_eq!(*actual, 0);
    }

    #[tokio::test]
    async fn test_with_multi_async_get() {
        let cache = Arc::new(Dedupe::<u64, u64>::new(1000, true));

        let a = cache.dedupe(&1, || {
            Box::pin(async move {
                sleep(Duration::from_millis(1)).await;
                Arc::new(1)
            })
        });
        let b = cache.dedupe(&1, || {
            Box::pin(async move {
                sleep(Duration::from_millis(1)).await;
                Arc::new(2)
            })
        });
        let (a, b) = join!(a, b);

        assert_eq!(a, b);
    }

    async fn compute_value(counter: Arc<AtomicUsize>) -> Arc<String> {
        counter.fetch_add(1, Ordering::SeqCst);
        sleep(Duration::from_millis(1)).await;
        Arc::new(format!("value_{}", counter.load(Ordering::SeqCst)))
    }

    #[tokio::test(worker_threads = 16, flavor = "multi_thread")]
    async fn test_deadlock_scenario() {
        let _ = tracing_subscriber::fmt();
        let cache = Arc::new(Dedupe::<u64, String>::new(1000, true));
        let key = 1;
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();

        // Spawn multiple tasks to simulate concurrent access
        for i in 0..1000000 {
            let cache = cache.clone();
            let counter = counter.clone();
            let handle = tokio::task::spawn(async move {
                let result = cache
                    .dedupe(&key, || Box::pin(compute_value(counter)))
                    .await;
                (i, result)
            });
            handles.push(handle);
        }
        // Await each task for any potential deadlocks
        for handle in handles.into_iter() {
            let _ = handle.await.unwrap();
        }
        // Check that compute_value was called exactly once
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "compute_value was called more than once"
        );
    }

    #[tokio::test]
    async fn test_hanging_after_dropped() {
        let cache = Arc::new(Dedupe::<u64, ()>::new(100, true));

        let task = cache.dedupe(&1, move || async move {
            sleep(Duration::from_millis(100)).await;
            Arc::new(())
        });

        // drops the task since the underlying sleep timeout is higher than the
        // timeout here

        timeout_at(Instant::now() + Duration::from_millis(10), task)
            .await
            .expect_err("Should throw timeout error");

        cache
            .dedupe(&1, move || async move {
                sleep(Duration::from_millis(100)).await;
                Arc::new(())
            })
            .await;
    }

    #[tokio::test]
    async fn test_hanging_dropped_while_in_use() {
        let cache = Arc::new(Dedupe::<u64, u64>::new(100, true));
        let cache_1 = cache.clone();
        let cache_2 = cache.clone();

        let task_1 = tokio::spawn(async move {
            cache_1
                .dedupe(&1, move || async move {
                    sleep(Duration::from_millis(100)).await;
                    Arc::new(100)
                })
                .await
        });

        let task_2 = tokio::spawn(async move {
            cache_2
                .dedupe(&1, move || async move {
                    sleep(Duration::from_millis(100)).await;
                    Arc::new(200)
                })
                .await
        });

        sleep(Duration::from_millis(10)).await;

        // drop the first task
        task_1.abort();

        let actual = task_2.await.unwrap();
        assert_eq!(*actual, 200)
    }

    // TODO: This is a failing test
    #[tokio::test]
    #[ignore]
    async fn test_should_not_abort_call_1() {
        #[derive(Debug, PartialEq, Clone)]
        struct Status {
            // Set this in the first call
            call_1: bool,

            // Set this in the second call
            call_2: bool,
        }

        let status = Arc::new(Mutex::new(Status { call_1: false, call_2: false }));

        let cache = Arc::new(Dedupe::<u64, ()>::new(100, true));
        let cache_1 = cache.clone();
        let cache_2 = cache.clone();
        let status_1 = status.clone();
        let status_2 = status.clone();

        // Task 1 completed in 100ms
        let task_1 = tokio::spawn(async move {
            cache_1
                .dedupe(&1, move || async move {
                    sleep(Duration::from_millis(100)).await;
                    status_1.lock().unwrap().call_1 = true;
                    Arc::new(())
                })
                .await
        });

        // Wait for 10ms
        sleep(Duration::from_millis(10)).await;

        // Task 2 completed in 200ms
        tokio::spawn(async move {
            cache_2
                .dedupe(&1, move || async move {
                    sleep(Duration::from_millis(120)).await;
                    status_2.lock().unwrap().call_2 = true;
                    Arc::new(())
                })
                .await
        });

        // Wait for 10ms
        sleep(Duration::from_millis(10)).await;

        // Abort the task_1
        task_1.abort();

        sleep(Duration::from_millis(300)).await;

        // Task 1 should still have completed because others are dependent on it.
        let actual = status.lock().unwrap().deref().to_owned();
        assert_eq!(actual, Status { call_1: true, call_2: false })
    }

    #[tokio::test]
    async fn test_should_abort_all() {
        #[derive(Debug, PartialEq, Clone)]
        struct Status {
            // Set this in the first call
            call_1: bool,

            // Set this in the second call
            call_2: bool,
        }

        let status = Arc::new(Mutex::new(Status { call_1: false, call_2: false }));

        let cache = Arc::new(Dedupe::<u64, ()>::new(100, true));
        let cache_1 = cache.clone();
        let cache_2 = cache.clone();
        let status_1 = status.clone();
        let status_2 = status.clone();

        // Task 1 completed in 100ms
        let task_1 = tokio::spawn(async move {
            cache_1
                .dedupe(&1, move || async move {
                    sleep(Duration::from_millis(100)).await;
                    status_1.lock().unwrap().call_1 = true;
                    Arc::new(())
                })
                .await
        });

        // Task 2 completed in 150ms
        let task_2 = tokio::spawn(async move {
            cache_2
                .dedupe(&1, move || async move {
                    sleep(Duration::from_millis(150)).await;
                    status_2.lock().unwrap().call_2 = true;
                    Arc::new(())
                })
                .await
        });

        // Wait for 10ms
        sleep(Duration::from_millis(50)).await;

        // Abort the task_1 & task_2
        task_1.abort();
        task_2.abort();

        sleep(Duration::from_millis(300)).await;

        // No task should have completed
        let actual = status.lock().unwrap().deref().to_owned();
        assert_eq!(actual, Status { call_1: false, call_2: false })
    }
}
