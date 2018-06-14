extern crate actor;
extern crate futures_core;

use actor::actor::actor_fn;
use futures_core::Async;

mod util;

use util::quick_handle;

#[test]
fn test_actor_fn() {
    let mut actor_value = 0;
    {
        let mut actor = actor_fn(|value: usize| -> Result<(), ()> {
            actor_value += value;
            Ok(())
        });

        assert_eq!(quick_handle(&mut actor, 1), Ok(Async::Ready(())));
        assert_eq!(quick_handle(&mut actor, 10), Ok(Async::Ready(())));
    }
    assert_eq!(actor_value, 11);
}
