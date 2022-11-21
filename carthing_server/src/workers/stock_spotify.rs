use anyhow::Context;
use anyhow::Result;
use serde_json::json;
use std::io::Read;
use std::io::Write;

pub struct SpotifyConnectionWorker {
    is_authed: bool,
}

impl SpotifyConnectionWorker {
    pub fn new() -> Self {
        Self { is_authed: false }
    }

    pub fn run(mut self, mut sock: impl Read + Write) -> Result<()> {
        loop {
            let mut msg_len = [0; 4];
            if let Err(e) = sock.read_exact(&mut msg_len) {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    println!("device disconnected");
                    return Ok(());
                } else {
                    return Err(e.into());
                }
            }

            // TODO: handling wamp connection state + packets certainly seems
            // like something that's best left to a library...

            let msg = rmpv::decode::read_value(&mut sock)?;
            println!("recv <-- {}|{:#}", u32::from_be_bytes(msg_len), msg);

            let rmpv::Value::Array(msg) = msg else {
                anyhow::bail!("unexpected msg: expected array")
            };

            let mut outgoing = Vec::new();
            let res = self
                .process_packet(msg)
                .context("while processing packet")?;
            rmp_serde::encode::write_named(&mut outgoing, &res)?;

            {
                let v = rmpv::decode::read_value(&mut std::io::Cursor::new(outgoing.clone()))?;
                println!("send --> {}|{:#}", outgoing.len(), v);
            }

            sock.write_all(&(outgoing.len() as u32).to_be_bytes())?;
            sock.write_all(&outgoing)?;
            outgoing.clear();
        }
    }

    fn handle_auth(&mut self, msg: Vec<rmpv::Value>) -> Result<serde_json::Value> {
        let dt = chrono::Local::now();
        let timestamp = dt.format("%FT%T").to_string();

        let id = msg[0].as_u64().unwrap();
        let res = match id {
            // HELLO
            1 => {
                let challenge = json!({
                    "authid": msg[2]["authid"].as_str().unwrap(),
                    "authmethod": "wampcra",
                    "authprovider": "spotify",
                    "authrole": "app",
                    "nonce": "dummy_nonce",
                    "session": 0,
                    "timestamp": timestamp,
                });

                // bruh
                let challenge = serde_json::to_string(&challenge)?;

                json!([
                    4, // CHALLENGE
                    "wampcra",
                    { "challenge": challenge }
                ])
            }
            // AUTHENTICATE
            5 => {
                // don't actually verify the challenge
                let wamp_session_id = 1; // supposed to be random

                json!([
                    2, // WELCOME
                    wamp_session_id,
                    {
                        "app_version": "8.7.82.94",
                        "date_time": timestamp,
                        "roles": {
                            "broker": {},
                            "dealer": {}
                        }
                    }
                ])
            }
            _ => todo!(),
        };
        Ok(res)
    }

    fn process_packet(&mut self, msg: Vec<rmpv::Value>) -> Result<serde_json::Value> {
        if !self.is_authed {
            self.handle_auth(msg)
        } else {
            // TODO
            todo!()
        }
    }
}
