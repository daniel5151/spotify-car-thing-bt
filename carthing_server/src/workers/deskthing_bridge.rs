use anyhow::Context;
use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;

pub struct DeskthingWorkerHandles {
    bridge_rx_worker: std::thread::JoinHandle<()>,
    bridge_tx_worker: std::thread::JoinHandle<()>,
}

impl DeskthingWorkerHandles {
    pub fn wait_for_shutdown(self) -> anyhow::Result<()> {
        if self.bridge_rx_worker.join().is_err() {
            println!("bridge_rx_worker panicked!")
        }

        if self.bridge_tx_worker.join().is_err() {
            println!("bridge_tx_worker panicked!")
        }

        Ok(())
    }
}

pub fn spawn_deskthing_bridge_workers(
    ws_tx: Sender<serde_json::Value>,
    ws_rx: Receiver<serde_json::Value>,
    topic_tx: Sender<(String, serde_json::Value, usize)>,
) -> anyhow::Result<DeskthingWorkerHandles> {
    let bridge_rx_worker = {
        std::thread::spawn(move || {
            if let Err(e) = DeskthingRxWorker::new(ws_rx, topic_tx).run() {
                println!("deskthing rx error: {:?}", e);
                std::process::abort();
            }
        })
    };

    let bridge_tx_worker = {
        std::thread::spawn(move || {
            if let Err(e) = DeskthingTxWorker::new(ws_tx).run() {
                println!("deskthing tx error: {:?}", e);
                std::process::abort();
            }
        })
    };

    Ok(DeskthingWorkerHandles {
        bridge_rx_worker,
        bridge_tx_worker,
    })
}

struct DeskthingRxWorker {
    ws_rx: Receiver<serde_json::Value>,
    topic_tx: Sender<(String, serde_json::Value, usize)>,
}

impl DeskthingRxWorker {
    fn new(
        ws_rx: Receiver<serde_json::Value>,
        topic_tx: Sender<(String, serde_json::Value, usize)>,
    ) -> Self {
        Self { ws_rx, topic_tx }
    }

    fn run(self) -> anyhow::Result<()> {
        let mut next_pub_id = 1;

        loop {
            let mut msg = self.ws_rx.recv()?;

            let topic = msg["topic"]
                .as_str()
                .context(format!("invalid topic kind: {}", msg["topic"]))?;

            let pub_id = next_pub_id;
            next_pub_id += 1;
            self.topic_tx
                .send((topic.to_owned(), msg["state"].take(), pub_id))?;
        }
    }
}

struct DeskthingTxWorker {
    ws_tx: Sender<serde_json::Value>,
}

impl DeskthingTxWorker {
    fn new(ws_tx: Sender<serde_json::Value>) -> Self {
        Self { ws_tx }
    }

    fn run(self) -> anyhow::Result<()> {
        loop {
            // TEMP
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}
