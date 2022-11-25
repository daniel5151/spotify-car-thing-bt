use anyhow::Result;
use uuid::Uuid;

mod sys;
mod workers;

const GUID_SPOTIFY: Uuid = Uuid::from_fields(
    0xe3cccccd,
    0x33b7,
    0x457d,
    &[0xa0, 0x3c, 0xaa, 0x1c, 0x54, 0xbf, 0x61, 0x7f],
);

fn accept_bt_connections() -> Result<()> {
    let mut socket = sys::BtSocketListener::bind()?;
    println!("Listening on RFCOMM port {}", socket.rfcomm_port());

    socket.register_service("Spotify Car Thing", GUID_SPOTIFY)?;

    loop {
        let sock = socket.accept()?;
        println!(
            "Connection received from {:04x}{:08x} to port {}",
            sock.nap(),
            sock.sap(),
            sock.port()
        );

        let carthing_client = workers::stock_spotify::CarthingClient::new(
            Box::new(sock.try_clone()?),
            Box::new(sock),
        )?;

        if carthing_client.wait_for_shutdown().is_err() {
            println!("could not cleanly join workers")
        }
    }
}

fn main() -> Result<()> {
    sys::platform_init()?;

    if let Err(e) = accept_bt_connections() {
        println!("error: {:?}", e);
    }

    sys::platform_teardown()?;
    Ok(())
}
