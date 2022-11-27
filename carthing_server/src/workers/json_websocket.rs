use anyhow::Context;
use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use std::fmt::Debug;
use std::io::Read;
use std::io::Write;
use tungstenite::WebSocket;

pub trait TryCloneIo: Send + Sync + Read + Write + Sized + Debug + 'static {
    fn try_clone(&self) -> anyhow::Result<Self>;
}

impl TryCloneIo for std::net::TcpStream {
    fn try_clone(&self) -> anyhow::Result<Self> {
        Ok(self.try_clone()?)
    }
}

pub fn spawn_json_websocket_workers(
    stream: impl TryCloneIo,
) -> anyhow::Result<(
    JsonWebsocketServerHandles,
    Sender<serde_json::Value>,
    Receiver<serde_json::Value>,
)> {
    let rx_stream = io_guard::PanicOnWrite(stream.try_clone()?);
    let tx_stream = io_guard::PanicOnRead(stream.try_clone()?);

    let _ws = tungstenite::accept(stream).context("creating websocket")?;

    let ws_rx = WebSocket::from_raw_socket(rx_stream, tungstenite::protocol::Role::Server, None);
    let ws_tx = WebSocket::from_raw_socket(tx_stream, tungstenite::protocol::Role::Server, None);

    let (in_tx, in_rx) = crossbeam_channel::unbounded();
    let (out_tx, out_rx) = crossbeam_channel::unbounded();

    let ws_rx_worker = std::thread::spawn(move || {
        if let Err(e) = JsonWebsocketRxWorker::new(ws_rx, in_tx).run() {
            println!("ws recv error: {:?}", e);
        }
    });

    let ws_tx_worker = std::thread::spawn(move || {
        if let Err(e) = JsonWebsocketTxWorker::new(ws_tx, out_rx).run() {
            println!("ws recv error: {:?}", e);
        }
    });

    Ok((
        JsonWebsocketServerHandles {
            ws_rx_worker,
            ws_tx_worker,
        },
        out_tx,
        in_rx,
    ))
}

pub struct JsonWebsocketServerHandles {
    ws_rx_worker: std::thread::JoinHandle<()>,
    ws_tx_worker: std::thread::JoinHandle<()>,
}

impl JsonWebsocketServerHandles {
    pub fn wait_for_shutdown(self) -> anyhow::Result<()> {
        if self.ws_rx_worker.join().is_err() {
            println!("ws_rx_worker panicked!")
        }

        if self.ws_tx_worker.join().is_err() {
            println!("ws_tx_worker panicked!")
        }

        Ok(())
    }
}

struct JsonWebsocketRxWorker<S> {
    ws: WebSocket<S>,
    in_tx: Sender<serde_json::Value>,
}

impl<S: Read + Write> JsonWebsocketRxWorker<S> {
    fn new(ws: WebSocket<S>, on_recv: Sender<serde_json::Value>) -> Self {
        Self { ws, in_tx: on_recv }
    }

    fn run(mut self) -> anyhow::Result<()> {
        loop {
            let msg = match self.ws.read_message()? {
                tungstenite::Message::Text(s) => serde_json::from_str(&s)?,
                tungstenite::Message::Binary(v) => serde_json::from_slice(&v)?,
                other => anyhow::bail!("unexpected ws message {}", other),
            };
            self.in_tx.send(msg).context("could not send msg")?;
        }
    }
}

struct JsonWebsocketTxWorker<S> {
    ws: WebSocket<S>,
    recv: Receiver<serde_json::Value>,
}

impl<S: Read + Write> JsonWebsocketTxWorker<S> {
    fn new(ws: WebSocket<S>, recv: Receiver<serde_json::Value>) -> Self {
        Self { ws, recv }
    }

    fn run(mut self) -> anyhow::Result<()> {
        loop {
            let msg = self.recv.recv()?;
            self.ws
                .write_message(tungstenite::Message::Text(msg.to_string()))?;
        }
    }
}

mod io_guard {
    use std::io::Read;
    use std::io::Result;
    use std::io::Write;

    pub struct PanicOnWrite<S>(pub S);
    impl<S: Read> Read for PanicOnWrite<S> {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            self.0.read(buf)
        }
    }
    impl<S: Write> Write for PanicOnWrite<S> {
        fn write(&mut self, _buf: &[u8]) -> Result<usize> {
            panic!("called write")
        }

        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    pub struct PanicOnRead<S>(pub S);
    impl<S> Read for PanicOnRead<S> {
        fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
            panic!("called read")
        }
    }
    impl<S: Write> Write for PanicOnRead<S> {
        fn write(&mut self, buf: &[u8]) -> Result<usize> {
            self.0.write(buf)
        }

        fn flush(&mut self) -> Result<()> {
            self.0.flush()
        }
    }
}
