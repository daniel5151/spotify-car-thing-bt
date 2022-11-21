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

    'outer: loop {
        let mut sock = socket.accept()?;
        println!(
            "Connection received from {:04x}{:08x} to port {}",
            sock.nap(),
            sock.sap(),
            sock.port()
        );

        loop {
            let mut msg_len = [0; 4];
            if let Err(e) = sock.read_exact(&mut msg_len) {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    println!("device disconnected");
                    continue 'outer;
                } else {
                    return Err(e.into());
                }
            }

            let msg_len = u32::from_be_bytes(msg_len);
            println!("msg len: {}", msg_len);

            let v = rmpv::decode::read_value(&mut sock)?;
            println!("received: {:#}", v);
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
