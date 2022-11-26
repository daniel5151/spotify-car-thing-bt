use crate::workers::json_websocket::spawn_json_websocket_server;
use crate::workers::stock_spotify::spawn_car_thing_server;
use crate::workers::stock_spotify::CarThingServerChans;
use anyhow::Context;
use anyhow::Result;
use std::net::TcpListener;
use uuid::Uuid;

const DESKTHING_PORT: u16 = 36308;
const GUID_SPOTIFY: Uuid = Uuid::from_fields(
    0xe3cccccd,
    0x33b7,
    0x457d,
    &[0xa0, 0x3c, 0xaa, 0x1c, 0x54, 0xbf, 0x61, 0x7f],
);

pub fn run_deskthing() -> Result<()> {
    // set up websocket
    let ws_server = TcpListener::bind(format!("127.0.0.1:{DESKTHING_PORT}"))
        .context(format!("binding to ws port {}", DESKTHING_PORT))?;

    // set up bluetooth
    let mut bt_socket = crate::sys::BtSocketListener::bind().context("binding to bt port")?;
    bt_socket
        .register_service("Spotify Car Thing", GUID_SPOTIFY)
        .context("registering bt service")?;

    // for now, only support 1:1 spotify client <-> carthing
    loop {
        let bt_sock = {
            println!(
                "waiting for bt connection on RFCOMM port {}...",
                bt_socket.rfcomm_port()
            );
            let bt_sock = bt_socket.accept()?;
            println!(
                "Connection received from {:04x}{:08x} to port {}",
                bt_sock.nap(),
                bt_sock.sap(),
                bt_sock.port()
            );
            bt_sock
        };

        let ws_stream = {
            println!("waiting for ws connection on 127.0.0.1:{DESKTHING_PORT}...");
            let (ws_stream, ws_addr) = ws_server.accept().context("accepting connection")?;
            println!("accepted ws connection from {}", ws_addr);
            ws_stream
        };

        let (ws_server, ws_tx, ws_rx) =
            spawn_json_websocket_server(ws_stream).context("constructing ws server")?;

        let (car_thing_server, CarThingServerChans { topic_tx }) =
            spawn_car_thing_server(Box::new(bt_sock.try_clone()?), Box::new(bt_sock))
                .context("constructing carthing client")?;

        // let the servers run
        {
            if ws_server.wait_for_shutdown().is_err() {
                println!("ws_server did not shut down cleanly")
            }

            if car_thing_server.wait_for_shutdown().is_err() {
                println!("car_thing_server did not shut down cleanly")
            }
        }
    }
}
