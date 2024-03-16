use heph::messages::Terminate;
use heph_rt::Signal;

#[test]
fn terminate_try_from_signal() {
    let tests = [
        (Signal::Interrupt, Ok(Terminate)),
        (Signal::Terminate, Ok(Terminate)),
        (Signal::Quit, Ok(Terminate)),
        (Signal::User1, Err(())),
        (Signal::User2, Err(())),
    ];

    for (signal, expected) in tests {
        let got = Terminate::try_from(signal);
        assert_eq!(expected, got);
    }
}
