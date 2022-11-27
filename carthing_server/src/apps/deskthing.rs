use crate::workers::deskthing_bridge::spawn_deskthing_bridge_workers;
use crate::workers::deskthing_bridge::DeskthingChans;
use crate::workers::json_websocket::spawn_json_websocket_workers;
use crate::workers::stock_spotify::spawn_car_thing_workers;
use crate::workers::stock_spotify::CarThingServerChans;
use anyhow::Context;
use anyhow::Result;
use std::convert::Infallible;
use std::net::TcpListener;
use uuid::Uuid;

const DESKTHING_PORT: u16 = 36308;
const GUID_SPOTIFY: Uuid = Uuid::from_fields(
    0xe3cccccd,
    0x33b7,
    0x457d,
    &[0xa0, 0x3c, 0xaa, 0x1c, 0x54, 0xbf, 0x61, 0x7f],
);

fn accept_car_thing(chans: DeskthingChans) -> anyhow::Result<Infallible> {
    let mut bt_socket = crate::sys::BtSocketListener::bind().context("binding to bt port")?;
    bt_socket
        .register_service("Spotify Car Thing", GUID_SPOTIFY)
        .context("registering bt service")?;

    loop {
        let bt_sock = {
            println!(
                "waiting for bt connection on RFCOMM port {}...",
                bt_socket.rfcomm_port()
            );
            let bt_sock = bt_socket.accept().context("accepting bt connection")?;
            println!(
                "Connection received from {:04x}{:08x} to port {}",
                bt_sock.nap(),
                bt_sock.sap(),
                bt_sock.port()
            );
            bt_sock
        };

        let (
            car_thing_server,
            CarThingServerChans {
                topic_tx,
                rpc_req_rx,
                rpc_res_tx,
                state_req_rx,
            },
        ) = spawn_car_thing_workers(Box::new(bt_sock.try_clone()?), Box::new(bt_sock))
            .context("constructing carthing client")?;

        chans
            .update_bt(topic_tx, state_req_rx, rpc_req_rx, rpc_res_tx)
            .context("trying to update bt wiring")?;

        if car_thing_server.wait_for_shutdown().is_err() {
            println!("car_thing_server did not shut down cleanly")
        }
    }
}

fn accept_websocket(chans: DeskthingChans) -> anyhow::Result<Infallible> {
    let ws_server = TcpListener::bind(format!("127.0.0.1:{DESKTHING_PORT}"))
        .context(format!("binding to ws port {}", DESKTHING_PORT))?;

    loop {
        let ws_stream = {
            println!("waiting for ws connection on 127.0.0.1:{DESKTHING_PORT}...");
            let (ws_stream, ws_addr) = ws_server.accept().context("accepting ws connection")?;
            println!("accepted ws connection from {}", ws_addr);
            ws_stream
        };

        let (ws_server, ws_tx, ws_rx) =
            spawn_json_websocket_workers(ws_stream).context("constructing ws server")?;

        chans.update_ws(ws_tx, ws_rx)?;

        if ws_server.wait_for_shutdown().is_err() {
            println!("car_thing_server did not shut down cleanly")
        }
    }
}

pub fn run_deskthing() -> Result<()> {
    let (deskthing_server, chans) = spawn_deskthing_bridge_workers()?;

    let _accept_car_thing = {
        let chans = chans.clone();
        std::thread::spawn(move || {
            if let Err(e) = accept_car_thing(chans) {
                println!("failure accepting bt connection: {:?}", e)
            }
        })
    };

    let _accept_websocket = {
        let chans = chans;
        std::thread::spawn(move || {
            if let Err(e) = accept_websocket(chans) {
                println!("failure accepting ws connection: {:?}", e)
            }
        })
    };

    if deskthing_server.wait_for_shutdown().is_err() {
        println!("deskthing_server did not shut down cleanly")
    }

    Ok(())
}
