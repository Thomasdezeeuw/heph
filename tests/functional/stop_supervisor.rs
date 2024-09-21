use std::panic;

use heph::supervisor::{StopSupervisor, Supervisor, SupervisorStrategy, SyncSupervisor};

use crate::util::{EmptyActor, EmptyNewActor};

#[test]
fn stop_supervisor_decide() {
    assert_eq!(
        Supervisor::<EmptyNewActor>::decide(&mut StopSupervisor, "first error"),
        SupervisorStrategy::Stop
    );
}

#[test]
fn stop_supervisor_decide_on_restart_error() {
    assert_eq!(
        Supervisor::<EmptyNewActor>::decide_on_restart_error(&mut StopSupervisor, "restart error"),
        SupervisorStrategy::Stop
    );
}

#[test]
fn stop_supervisor_second_restart_error() {
    Supervisor::<EmptyNewActor>::second_restart_error(&mut StopSupervisor, "second restart error");
}

#[test]
fn stop_supervisor_decide_on_panic() {
    let panic = panic::catch_unwind(|| panic!("original panic")).unwrap_err();
    assert_eq!(
        Supervisor::<EmptyNewActor>::decide_on_panic(&mut StopSupervisor, panic),
        SupervisorStrategy::Stop
    );
}

#[test]
fn stop_supervisor_sync_decide() {
    assert_eq!(
        SyncSupervisor::<EmptyActor>::decide(&mut StopSupervisor, "first error"),
        SupervisorStrategy::Stop
    );
}

#[test]
fn stop_supervisor_sync_decide_on_panic() {
    let panic = panic::catch_unwind(|| panic!("original panic")).unwrap_err();
    assert_eq!(
        SyncSupervisor::<EmptyActor>::decide_on_panic(&mut StopSupervisor, panic),
        SupervisorStrategy::Stop
    );
}
