extern crate actor;
extern crate futures_core;

use actor::actor::actor_fn;
use futures_core::Async;

mod util;

use util::{quick_handle, quick_poll};

#[test]
fn test_actor_fn() {
    let mut actor_value = 0;
    {
        let mut returned_ok = false;
        let mut actor = actor_fn(|value: usize| {
            actor_value += value;
            if !returned_ok {
                returned_ok = true;
                Ok(())
            } else {
                Err(())
            }
        });

        assert_eq!(quick_handle(&mut actor, 1), Ok(Async::Ready(())));
        assert_eq!(quick_handle(&mut actor, 10), Err(()));
        assert_eq!(quick_poll(&mut actor), Ok(Async::Ready(())));
    }
    assert_eq!(actor_value, 11);
}
