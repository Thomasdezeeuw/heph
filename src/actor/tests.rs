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
        // Async function named actor in an actor module.
        (
            "core::future::from_generator::GenFuture<heph::actor::tests::storage::actor::actor::{{closure}}>",
            "storage::actor",
        ),
        // Type implementing `Actor`.
        (
            "heph::net::tcp::server::TcpServer<2_my_ip::conn_supervisor, fn(heph::actor::context_priv::Context<!>, heph::net::tcp::stream::TcpStream, std::net::addr::SocketAddr) -> core::future::from_generator::GenFuture<2_my_ip::conn_actor::{{closure}}>, heph::actor::context_priv::ThreadLocal>",
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
            "TcpServer<2_my_ip::conn_supervisor, fn(heph::actor::context_priv::Context<!>, heph::net::tcp::stream::TcpStream, std::net::addr::SocketAddr) -> core::future::from_generator::GenFuture<2_my_ip::conn_actor::{{closure}}>, heph::actor::context_priv::ThreadLocal>",
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
        assert_eq!(got, *expected);
    }
}
