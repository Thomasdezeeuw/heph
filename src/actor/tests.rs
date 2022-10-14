use crate::actor;

#[test]
fn actor_name() {
    let tests = &[
        (
            "core::future::from_generator::GenFuture<1_hello_world::greeter_actor::{{closure}}>",
            "greeter_actor"
        ),
        (
            "core::future::from_generator::GenFuture<1_hello_world::some::nested::module::greeter::actor::{{closure}}>",
            "greeter::actor",
        ),
        (
            "core::future::from_generator::GenFuture<3_rpc::ping_actor>",
            "ping_actor"
        ),
        (
            "core::future::from_generator::GenFuture<2_my_ip::conn_actor::{{closure}}>",
            "conn_actor"
        ),
        // Generic parameter(s) wrapped in GenFuture.
        (
            "core::future::from_generator::GenFuture<functional::functional::timer::triggered_timers_run_actors::deadline_actor<heph::actor::context::ThreadLocal>::{{closure}}>",
            "deadline_actor",
        ),
        (
            "core::future::from_generator::GenFuture<functional::functional::timer::triggered_timers_run_actors::timer_actor<heph::actor::context::ThreadSafe>::{{closure}}>",
            "timer_actor"
        ),
        (
            "core::future::from_generator::GenFuture<my_actor<P1>::{{closure}}>",
            "my_actor",
        ),
        (
            "core::future::from_generator::GenFuture<my_actor<P1, P2>::{{closure}}>",
            "my_actor",
        ),
        (
            "core::future::from_generator::GenFuture<my_actor<P1<P1A>, P2>::{{closure}}>",
            "my_actor",
        ),
        (
            "core::future::from_generator::GenFuture<my_actor<P1<P1A>, P2<P2A>>::{{closure}}>",
            "my_actor",
        ),
        (
            "core::future::from_generator::GenFuture<my_actor<P1<P1A<P1B>>, P2<P2A>>::{{closure}}>",
            "my_actor",
        ),
        // Async function named actor in an actor module.
        (
            "core::future::from_generator::GenFuture<heph::actor::tests::storage::actor::actor::{{closure}}>",
            "storage::actor",
        ),
        // Type implementing `Actor`.
        (
            "heph::net::tcp::server::TcpServer<2_my_ip::conn_supervisor, fn(heph::actor::context::Context<!>, heph::net::tcp::stream::TcpStream, std::net::addr::SocketAddr) -> core::future::from_generator::GenFuture<2_my_ip::conn_actor::{{closure}}>, heph::actor::context::ThreadLocal>",
            "TcpServer",
        ),
        // If the module path is removed from `std::any::type_name`.
        (
            "GenFuture<1_hello_world::greeter_actor::{{closure}}>",
            "greeter_actor"
        ),
        (
            "GenFuture<1_hello_world::some::nested::module::greeter::actor::{{closure}}>",
            "greeter::actor",
        ),
        (
            "TcpServer<2_my_ip::conn_supervisor, fn(heph::actor::context::Context<!>, heph::net::tcp::stream::TcpStream, std::net::addr::SocketAddr) -> core::future::from_generator::GenFuture<2_my_ip::conn_actor::{{closure}}>, heph::actor::context::ThreadLocal>",
            "TcpServer",
        ),
        (
            "TcpServer<core::future::from_generator::GenFuture<2_my_ip::conn_actor::{{closure}}>>",
            "TcpServer",
        ),
        (
            "TcpServer<GenFuture<2_my_ip::conn_actor::{{closure}}>>",
            "TcpServer",
        ),
    ];

    for (input, expected) in tests {
        let got = actor::format_name(input);
        assert_eq!(got, *expected, "input: {input}");
    }
}
