//! The one process-wide tokio runtime every exported method runs on.

use std::any::Any;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("reins-core")
            .enable_all()
            .build()
            .expect("failed to start the reins-core runtime")
    })
}

/// Runs `fut` to completion on the core runtime and returns its output.
///
/// The work is spawned: dropping the returned future (Kotlin cancelling the
/// coroutine) stops waiting for it but never aborts the work itself, so an
/// approval or a long-poll that already reached the server always finishes.
pub async fn run<F>(fut: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match runtime().spawn(fut).await {
        Ok(output) => output,
        Err(e) => {
            let payload: Box<dyn Any + Send> = match e.try_into_panic() {
                Ok(payload) => payload,
                Err(e) => Box::new(format!("reins-core task failed: {e}")),
            };
            std::panic::resume_unwind(payload)
        }
    }
}

/// Spawns detached background work on the core runtime.
pub fn spawn<F>(fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    drop(runtime().spawn(fut));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Waker};
    use std::time::Duration;

    use super::*;

    #[test]
    fn run_works_without_an_ambient_runtime() {
        let v = futures::executor::block_on(run(async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            7
        }));
        assert_eq!(v, 7);
    }

    #[test]
    fn dropping_the_caller_does_not_abort_the_work() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        let mut fut = Box::pin(run(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            flag.store(true, Ordering::SeqCst);
        }));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(fut.as_mut().poll(&mut cx).is_pending(), "first poll spawns the task");
        drop(fut);
        std::thread::sleep(Duration::from_millis(400));
        assert!(done.load(Ordering::SeqCst), "work finished although the caller went away");
    }

    #[test]
    fn spawn_runs_detached() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        spawn(async move { flag.store(true, Ordering::SeqCst) });
        std::thread::sleep(Duration::from_millis(200));
        assert!(done.load(Ordering::SeqCst));
    }

    #[test]
    #[should_panic(expected = "boom")]
    fn panics_propagate_to_the_caller() {
        futures::executor::block_on(run(async { panic!("boom") }));
    }
}
