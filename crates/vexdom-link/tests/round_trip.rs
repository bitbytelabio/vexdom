//! `Control.status` through a real loopback WebSocket.

use std::future::Future;

use vexdom_link::{WsStream, connect_control};
use vexdom_schema::control;

struct FakeControl;

impl control::Server for FakeControl {
    fn status(
        self: capnp::capability::Rc<Self>,
        _: control::StatusParams,
        mut results: control::StatusResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let mut status = results.get().init_status();
        status.set_connected(true);
        status.set_daemon_version("0.9.2");
        status.set_port(4242);
        status.reborrow().init_pairing().set_mode("auto");
        std::future::ready(Ok(()))
    }
}

#[test]
#[allow(clippy::result_large_err)]
fn status_round_trips() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::task::spawn_local(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let callback =
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.headers()["authorization"], "Bearer secret");
                    Ok(response)
                };
            let ws = tokio_tungstenite::accept_hdr_async(tcp, callback)
                .await
                .unwrap();
            let bootstrap: control::Client = capnp_rpc::new_client(FakeControl);
            let _ = vexdom_link::serve(WsStream::new(ws), bootstrap.client).await;
        });
        let (client, _handle) = connect_control(port, "secret").await.unwrap();
        let reply = client.status_request().send().promise.await.unwrap();
        let status = reply.get().unwrap().get_status().unwrap();
        assert!(status.get_connected());
        assert_eq!(status.get_port(), 4242);
        assert_eq!(
            status.get_daemon_version().unwrap().to_str().unwrap(),
            "0.9.2"
        );
        assert_eq!(
            status.get_pairing().get_mode().unwrap().to_str().unwrap(),
            "auto"
        );
    });
}
