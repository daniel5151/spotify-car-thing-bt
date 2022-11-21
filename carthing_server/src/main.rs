use bstr::ByteSlice;
use std::io::Read;
use uuid::Uuid;

mod sys;

const GUID_SPOTIFY: Uuid = Uuid::from_fields(
    0xe3cccccd,
    0x33b7,
    0x457d,
    &[0xa0, 0x3c, 0xaa, 0x1c, 0x54, 0xbf, 0x61, 0x7f],
);

fn accept_connections() -> anyhow::Result<()> {
    let mut socket = sys::BtSocketListener::bind()?;
    println!("Listening on RFCOMM port {}", socket.rfcomm_port());

    socket.register_service("Spotify Car Thing", GUID_SPOTIFY)?;

    loop {
        let mut sock = socket.accept()?;
        println!(
            "Connection received from {:04x}{:08x} to port {}",
            sock.nap(),
            sock.sap(),
            sock.port()
        );

        loop {
            let mut buf = [0; 512];
            let len = sock.read(&mut buf)?;
            if len == 0 {
                println!("device disconnected");
                break;
            }
            println!("received: {:?}", buf[..len].as_bstr());
        }
    }
}

fn main() -> anyhow::Result<()> {
    sys::platform_init()?;

    if let Err(e) = accept_connections() {
        println!("error: {:?}", e);
    }

    sys::platform_teardown()?;
    Ok(())
}
