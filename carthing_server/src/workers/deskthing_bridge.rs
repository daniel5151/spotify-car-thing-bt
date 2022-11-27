use super::stock_spotify::CarThingRpcReq;
use super::stock_spotify::CarThingRpcRes;
use anyhow::Context;
use crossbeam_channel::select;
use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use serde_json::json;

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
    state_req_rx: Receiver<String>,
    rpc_req_rx: Receiver<CarThingRpcReq>,
    rpc_res_tx: Sender<CarThingRpcRes>,
) -> anyhow::Result<DeskthingWorkerHandles> {
    let bridge_rx_worker = {
        std::thread::spawn(move || {
            if let Err(e) = DeskthingRxWorker::new(ws_rx, topic_tx, rpc_res_tx).run() {
                println!("deskthing rx error: {:?}", e);
                std::process::abort();
            }
        })
    };

    let bridge_tx_worker = {
        std::thread::spawn(move || {
            if let Err(e) = DeskthingTxWorker::new(ws_tx, state_req_rx, rpc_req_rx).run() {
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
    rpc_res_tx: Sender<CarThingRpcRes>,
}

impl DeskthingRxWorker {
    fn new(
        ws_rx: Receiver<serde_json::Value>,
        topic_tx: Sender<(String, serde_json::Value, usize)>,
        rpc_res_tx: Sender<CarThingRpcRes>,
    ) -> Self {
        Self {
            ws_rx,
            topic_tx,
            rpc_res_tx,
        }
    }

    fn run(self) -> anyhow::Result<()> {
        let mut next_pub_id = 1;

        loop {
            let mut msg = self.ws_rx.recv()?;
            if msg.get("result").is_some() {
                // handle rpc response
                let msg = msg["result"].take();

                let default_details = serde_json::Value::Object(serde_json::Map::new());
                let default_args = serde_json::Value::Object(serde_json::Map::new());
                let default_kwargs = serde_json::Value::Object(serde_json::Map::new());

                let req_id = msg.get("reqId").context("missing reqID")?;
                let details = msg.get("details").unwrap_or(&default_details);
                let args = msg.get("args").unwrap_or(&default_args);
                let kwargs = msg.get("argskw").unwrap_or(&default_kwargs);

                let (req_id, details, args, kwargs) = match (
                    req_id.as_u64(),
                    details.as_object(),
                    Some(args),
                    kwargs.as_object(),
                ) {
                    (None, _, _, _) => anyhow::bail!("invalid reqId: {}", req_id),
                    (_, None, _, _) => anyhow::bail!("invalid details: {}", details),
                    (_, _, None, _) => anyhow::bail!("invalid args: {}", args),
                    (_, _, _, None) => anyhow::bail!("invalid kwargs: {}", kwargs),
                    (Some(req_id), Some(details), Some(args), Some(kwargs)) => {
                        (req_id, details, args, kwargs)
                    }
                };

                self.rpc_res_tx.send(CarThingRpcRes {
                    req_id,
                    details: details.to_owned(),
                    args: args.to_owned(),
                    kwargs: kwargs.to_owned(),
                })?;
            } else if msg.get("topic").is_some() {
                // handle publish
                let topic = msg["topic"]
                    .as_str()
                    .context(format!("invalid topic kind: {}", msg["topic"]))?;

                let pub_id = next_pub_id;
                next_pub_id += 1;
                self.topic_tx
                    .send((topic.to_owned(), msg["state"].take(), pub_id))?;
            } else {
                anyhow::bail!("unexpected deskthing msg: {}", msg)
            }
        }
    }
}

struct DeskthingTxWorker {
    ws_tx: Sender<serde_json::Value>,
    state_req_rx: Receiver<String>,
    rpc_req_rx: Receiver<CarThingRpcReq>,
}

impl DeskthingTxWorker {
    fn new(
        ws_tx: Sender<serde_json::Value>,
        state_req_rx: Receiver<String>,
        rpc_req_rx: Receiver<CarThingRpcReq>,
    ) -> Self {
        Self {
            ws_tx,
            state_req_rx,
            rpc_req_rx,
        }
    }

    fn run(self) -> anyhow::Result<()> {
        enum Event {
            Rpc(CarThingRpcReq),
            State(String),
        }

        loop {
            let event = select! {
                recv(self.rpc_req_rx) -> msg => Event::Rpc(msg?),
                recv(self.state_req_rx) -> msg => Event::State(msg?),
            };

            match event {
                Event::Rpc(rpc) => {
                    self.ws_tx.send(json!({
                        "devId": 1,
                        "reqId": rpc.req_id,
                        "proc": rpc.proc,
                        "args": rpc.args,
                        "argsKw": rpc.kwargs,
                    }))?;
                }
                Event::State(topic) => {
                    self.ws_tx.send(json!({ "topic": topic }))?;
                }
            }
        }
    }
}
