use std::sync::Arc;

use futures_core::{Future, Poll};
use futures_core::task::{Context, LocalMap, Waker, Wake};

struct NopWaker;

impl Wake for NopWaker {
    fn wake(_: &Arc<Self>) { }
}

/// Quickly poll the `future`, with an empty map, no-op waker and without a
/// spawner.
pub fn quick_poll<F: Future>(future: &mut F) -> Poll<F::Item, F::Error> {
    let mut map = LocalMap::new();
    let mut waker = Waker::from(Arc::new(NopWaker));
    let mut ctx = Context::without_spawn(&mut map, &mut waker);
    future.poll(&mut ctx)
}
