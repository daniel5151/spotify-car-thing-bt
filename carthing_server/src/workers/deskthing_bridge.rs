use super::stock_spotify::CarThingRpcReq;
use super::stock_spotify::CarThingRpcRes;
use anyhow::Context;
use crossbeam_channel::select;
use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use serde_json::json;
use std::convert::Infallible;

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

#[derive(Clone)]
pub struct DeskthingChans {
    rx_obtain_ws: Sender<DeskthingRxWs>,
    rx_obtain_bt: Sender<DeskthingRxBt>,
    tx_obtain_ws: Sender<DeskthingTxWs>,
    tx_obtain_bt: Sender<DeskthingTxBt>,
}

impl DeskthingChans {
    pub fn update_ws(
        &self,
        ws_tx: Sender<serde_json::Value>,
        ws_rx: Receiver<serde_json::Value>,
    ) -> anyhow::Result<()> {
        self.rx_obtain_ws.send(DeskthingRxWs { ws_rx })?;
        self.tx_obtain_ws.send(DeskthingTxWs { ws_tx })?;
        Ok(())
    }

    pub fn update_bt(
        &self,
        topic_tx: Sender<(String, serde_json::Value, usize)>,
        state_req_rx: Receiver<String>,
        rpc_req_rx: Receiver<CarThingRpcReq>,
        rpc_res_tx: Sender<CarThingRpcRes>,
    ) -> anyhow::Result<()> {
        self.rx_obtain_bt.send(DeskthingRxBt {
            rpc_res_tx,
            topic_tx,
        })?;
        self.tx_obtain_bt.send(DeskthingTxBt {
            rpc_req_rx,
            state_req_rx,
        })?;

        Ok(())
    }
}

pub fn spawn_deskthing_bridge_workers() -> anyhow::Result<(DeskthingWorkerHandles, DeskthingChans)>
{
    let (rx_obtain_ws_tx, rx_obtain_ws_rx) = crossbeam_channel::unbounded();
    let (rx_obtain_bt_tx, rx_obtain_bt_rx) = crossbeam_channel::unbounded();
    let (tx_obtain_ws_tx, tx_obtain_ws_rx) = crossbeam_channel::unbounded();
    let (tx_obtain_bt_tx, tx_obtain_bt_rx) = crossbeam_channel::unbounded();

    let bridge_rx_worker = {
        std::thread::spawn(move || {
            if let Err(e) = DeskthingRxWorker::new(rx_obtain_ws_rx, rx_obtain_bt_rx).run() {
                println!("deskthing rx error: {:?}", e);
            }
        })
    };

    let bridge_tx_worker = {
        std::thread::spawn(move || {
            if let Err(e) = DeskthingTxWorker::new(tx_obtain_ws_rx, tx_obtain_bt_rx).run() {
                println!("deskthing tx error: {:?}", e);
            }
        })
    };

    Ok((
        DeskthingWorkerHandles {
            bridge_rx_worker,
            bridge_tx_worker,
        },
        DeskthingChans {
            rx_obtain_ws: rx_obtain_ws_tx,
            rx_obtain_bt: rx_obtain_bt_tx,
            tx_obtain_ws: tx_obtain_ws_tx,
            tx_obtain_bt: tx_obtain_bt_tx,
        },
    ))
}

enum WhatFailed {
    Ws,
    Bt,
}

pub struct DeskthingRxWs {
    pub ws_rx: Receiver<serde_json::Value>,
}

pub struct DeskthingRxBt {
    pub topic_tx: Sender<(String, serde_json::Value, usize)>,
    pub rpc_res_tx: Sender<CarThingRpcRes>,
}

struct DeskthingRxWorker {
    rx_obtain_ws: Receiver<DeskthingRxWs>,
    rx_obtain_bt: Receiver<DeskthingRxBt>,
}

impl DeskthingRxWorker {
    fn new(obtain_ws: Receiver<DeskthingRxWs>, obtain_bt: Receiver<DeskthingRxBt>) -> Self {
        Self {
            rx_obtain_ws: obtain_ws,
            rx_obtain_bt: obtain_bt,
        }
    }

    fn inner(
        &mut self,
        ws_rx: &Receiver<serde_json::Value>,
        topic_tx: &Sender<(String, serde_json::Value, usize)>,
        rpc_res_tx: &Sender<CarThingRpcRes>,
    ) -> anyhow::Result<WhatFailed> {
        let mut next_pub_id = 1;

        loop {
            let mut msg = match ws_rx.recv() {
                Ok(msg) => msg,
                Err(_) => return Ok(WhatFailed::Ws),
            };

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

                let res = rpc_res_tx.send(CarThingRpcRes {
                    req_id,
                    details: details.to_owned(),
                    args: args.to_owned(),
                    kwargs: kwargs.to_owned(),
                });

                if res.is_err() {
                    return Ok(WhatFailed::Bt);
                }
            } else if msg.get("topic").is_some() {
                // handle publish
                let topic = msg["topic"]
                    .as_str()
                    .context(format!("invalid topic kind: {}", msg["topic"]))?;

                let pub_id = next_pub_id;
                next_pub_id += 1;
                let res = topic_tx.send((topic.to_owned(), msg["state"].take(), pub_id));

                if res.is_err() {
                    return Ok(WhatFailed::Bt);
                }
            } else {
                anyhow::bail!("unexpected deskthing msg: {}", msg)
            }
        }
    }

    fn run(mut self) -> anyhow::Result<Infallible> {
        let mut ws = None;
        let mut bt = None;

        loop {
            while !(ws.is_some() && bt.is_some()) {
                select! {
                    recv(self.rx_obtain_ws) -> msg => ws = Some(msg?),
                    recv(self.rx_obtain_bt) -> msg => bt = Some(msg?),
                }
            }

            let DeskthingRxWs { ws_rx } = ws.as_ref().unwrap();
            let DeskthingRxBt {
                topic_tx,
                rpc_res_tx,
            } = bt.as_ref().unwrap();

            match self.inner(ws_rx, topic_tx, rpc_res_tx)? {
                WhatFailed::Ws => ws = None,
                WhatFailed::Bt => bt = None,
            }
        }
    }
}

pub struct DeskthingTxWs {
    pub ws_tx: Sender<serde_json::Value>,
}

pub struct DeskthingTxBt {
    pub state_req_rx: Receiver<String>,
    pub rpc_req_rx: Receiver<CarThingRpcReq>,
}

struct DeskthingTxWorker {
    tx_obtain_ws: Receiver<DeskthingTxWs>,
    tx_obtain_bt: Receiver<DeskthingTxBt>,
}

impl DeskthingTxWorker {
    fn new(obtain_ws: Receiver<DeskthingTxWs>, obtain_bt: Receiver<DeskthingTxBt>) -> Self {
        Self {
            tx_obtain_ws: obtain_ws,
            tx_obtain_bt: obtain_bt,
        }
    }

    fn inner(
        &mut self,
        ws_tx: &Sender<serde_json::Value>,
        state_req_rx: &Receiver<String>,
        rpc_req_rx: &Receiver<CarThingRpcReq>,
    ) -> anyhow::Result<WhatFailed> {
        enum Event {
            Rpc(CarThingRpcReq),
            State(String),
        }

        loop {
            let event = select! {
                recv(rpc_req_rx) -> msg => {
                    if msg.is_err() {
                        return Ok(WhatFailed::Bt)
                    }

                    Event::Rpc(msg?)
                },
                recv(state_req_rx) -> msg => {
                    if msg.is_err() {
                        return Ok(WhatFailed::Bt)
                    }

                    Event::State(msg?)
                },
            };

            let res = ws_tx.send(match event {
                Event::Rpc(rpc) => {
                    json!({
                        "devId": 1,
                        "reqId": rpc.req_id,
                        "proc": rpc.proc,
                        "args": rpc.args,
                        "argsKw": rpc.kwargs,
                    })
                }
                Event::State(topic) => {
                    json!({ "topic": topic })
                }
            });

            if res.is_err() {
                return Ok(WhatFailed::Ws);
            }
        }
    }

    fn run(mut self) -> anyhow::Result<Infallible> {
        let mut ws = None;
        let mut bt = None;

        loop {
            while !(ws.is_some() && bt.is_some()) {
                select! {
                    recv(self.tx_obtain_ws) -> msg => ws = Some(msg?),
                    recv(self.tx_obtain_bt) -> msg => bt = Some(msg?),
                }
            }

            let DeskthingTxWs { ws_tx } = ws.as_ref().unwrap();
            let DeskthingTxBt {
                state_req_rx,
                rpc_req_rx,
            } = bt.as_ref().unwrap();

            match self.inner(ws_tx, state_req_rx, rpc_req_rx)? {
                WhatFailed::Ws => ws = None,
                WhatFailed::Bt => bt = None,
            }
        }
    }
}
