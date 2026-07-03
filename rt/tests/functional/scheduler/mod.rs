use std::pin::Pin;
use std::task::{self, Poll};

use heph_rt::setup::scheduler::{FutureTask, Task};

mod cfs;
mod process_id;

#[test]
fn future_task() {
    struct MyTask;

    impl Future for MyTask {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<()> {
            unimplemented!();
        }
    }

    assert_eq!(FutureTask::new(MyTask).name(), "MyTask");
    assert_eq!(FutureTask::new(std::future::pending()).name(), "Pending");
}
