//! Cap'n Proto RPC over a WebSocket (ADR-018, ADR-025): every capnp message travels as binary
//! frames carrying an unaligned byte stream, so where a frame cuts the bytes does not matter. A text
//! frame is a protocol error; a close ends the stream. Runs on the current `LocalSet` (capnp
//! clients are `Rc`).

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use bytes::Bytes;
use capnp_rpc::rpc_twoparty_capnp::Side;
use capnp_rpc::{RpcSystem, twoparty};
use futures::io::{AsyncRead, AsyncWrite, BufWriter, ReadHalf, WriteHalf};
use futures::{AsyncReadExt as _, Sink, Stream};
use tokio_tungstenite::tungstenite::{self, Message};
use vexdom_schema::control;

/// What one WebSocket message means to the byte stream.
pub enum Incoming {
    Data(Bytes),
    /// Ping, pong and other frames that carry no capnp bytes.
    Ignore,
    /// A close frame: the stream ends.
    End,
    /// A text frame: never part of this protocol.
    Text,
}

/// A WebSocket message type the adapter can carry: tungstenite's, or axum's in the daemon.
pub trait Frame: Sized {
    fn binary(bytes: Vec<u8>) -> Self;
    fn classify(self) -> Incoming;
}

impl Frame for Message {
    fn binary(bytes: Vec<u8>) -> Self {
        Message::Binary(bytes.into())
    }

    fn classify(self) -> Incoming {
        match self {
            Message::Binary(bytes) => Incoming::Data(bytes),
            Message::Text(_) => Incoming::Text,
            Message::Close(_) => Incoming::End,
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => Incoming::Ignore,
        }
    }
}

/// A WebSocket as `futures` byte I/O: binary frames in, one binary frame per write out.
pub struct WsStream<S, M = Message> {
    inner: S,
    pending: Bytes,
    message: std::marker::PhantomData<fn() -> M>,
}

impl<S, M> WsStream<S, M> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            pending: Bytes::new(),
            message: std::marker::PhantomData,
        }
    }
}

impl<S, M, E> AsyncRead for WsStream<S, M>
where
    S: Stream<Item = Result<M, E>> + Unpin,
    M: Frame,
    E: std::fmt::Display,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            if !self.pending.is_empty() {
                let n = buf.len().min(self.pending.len());
                buf[..n].copy_from_slice(&self.pending.split_to(n));
                return Poll::Ready(Ok(n));
            }
            match ready!(Pin::new(&mut self.inner).poll_next(cx)) {
                None => return Poll::Ready(Ok(0)),
                Some(Err(error)) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        error.to_string(),
                    )));
                }
                Some(Ok(message)) => match message.classify() {
                    Incoming::Data(bytes) => self.pending = bytes,
                    Incoming::Ignore => {}
                    Incoming::End => return Poll::Ready(Ok(0)),
                    Incoming::Text => {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "a text frame on a Cap'n Proto socket",
                        )));
                    }
                },
            }
        }
    }
}

impl<S, M> AsyncWrite for WsStream<S, M>
where
    S: Sink<M> + Unpin,
    S::Error: std::fmt::Display,
    M: Frame,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        ready!(Pin::new(&mut self.inner).poll_ready(cx)).map_err(broken)?;
        Pin::new(&mut self.inner)
            .start_send(M::binary(buf.to_vec()))
            .map_err(broken)?;
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx).map_err(broken)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx).map_err(broken)
    }
}

fn broken(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, error.to_string())
}

/// Frames at most this big leave the write buffer; capnp flushes after each message.
const WRITE_BUFFER: usize = 1 << 16;

/// The two-party network over a byte stream, both ends' reader limits (ADR-018).
pub fn vat<T>(stream: T, side: Side) -> twoparty::VatNetwork<ReadHalf<T>>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    let (reader, writer): (ReadHalf<T>, WriteHalf<T>) = stream.split();
    twoparty::VatNetwork::new(
        reader,
        BufWriter::with_capacity(WRITE_BUFFER, writer),
        side,
        vexdom_schema::reader_options(),
    )
}

/// Serves `bootstrap` on a server-side stream; poll the returned system to run the connection.
pub fn serve<T>(stream: T, bootstrap: capnp::capability::Client) -> RpcSystem<Side>
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    RpcSystem::new(Box::new(vat(stream, Side::Server)), Some(bootstrap))
}

#[derive(Debug)]
pub enum LinkError {
    /// Nothing listens there, or the upgrade was refused (bad token).
    Connect(tungstenite::Error),
    /// The token cannot travel in a header.
    BadToken,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(error) => write!(f, "cannot reach Vexdom Bridge: {error}"),
            Self::BadToken => f.write_str("the control token is not a valid header value"),
        }
    }
}

impl std::error::Error for LinkError {}

/// Dials `ws://127.0.0.1:{port}/control` with the Bearer token and runs the RPC system on the
/// current `LocalSet`. The handle ends when the connection does.
pub async fn connect_control(
    port: u16,
    token: &str,
) -> Result<(control::Client, tokio::task::JoinHandle<()>), LinkError> {
    use tungstenite::client::IntoClientRequest as _;
    let mut request = format!("ws://127.0.0.1:{port}/control")
        .into_client_request()
        .map_err(LinkError::Connect)?;
    let bearer = format!("Bearer {token}")
        .parse()
        .map_err(|_| LinkError::BadToken)?;
    request.headers_mut().insert("authorization", bearer);
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(LinkError::Connect)?;
    let mut system = RpcSystem::new(Box::new(vat(WsStream::new(ws), Side::Client)), None);
    let client: control::Client = system.bootstrap(Side::Server);
    let handle = tokio::task::spawn_local(async move {
        let _ = system.await;
    });
    Ok((client, handle))
}

/// Dials an extension-role socket (`Origin` set), for tests and tools that act as the extension.
pub async fn connect_extension(
    port: u16,
    origin: &str,
) -> Result<(vexdom_schema::extension_gate::Client, RpcSystem<Side>), LinkError> {
    use tungstenite::client::IntoClientRequest as _;
    let mut request = format!("ws://127.0.0.1:{port}/extension")
        .into_client_request()
        .map_err(LinkError::Connect)?;
    if let Ok(value) = origin.parse() {
        request.headers_mut().insert("origin", value);
    }
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(LinkError::Connect)?;
    let mut system = RpcSystem::new(Box::new(vat(WsStream::new(ws), Side::Client)), None);
    let gate = system.bootstrap(Side::Server);
    Ok((gate, system))
}
