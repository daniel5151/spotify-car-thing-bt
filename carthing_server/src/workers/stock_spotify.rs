use anyhow::Context;
use anyhow::Result;
use serde_json::json;
use std::collections::HashMap;
use std::io::Read;
use std::io::Write;

pub struct SpotifyConnectionWorker<S> {
    sock: S,
    out_buf: Vec<u8>,

    is_authed: bool,

    next_sub_id: usize,
    subs: HashMap<String, Vec<usize>>,
}

impl<S: Read + Write> SpotifyConnectionWorker<S> {
    pub fn new(sock: S) -> Self {
        Self {
            sock,
            out_buf: Vec::new(),
            is_authed: false,
            next_sub_id: 1,
            subs: HashMap::new(),
        }
    }

    pub fn run(mut self) -> Result<()> {
        loop {
            let mut msg_len = [0; 4];
            if let Err(e) = self.sock.read_exact(&mut msg_len) {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    println!("device disconnected");
                    return Ok(());
                } else {
                    return Err(e.into());
                }
            }

            let msg = serde_transcode::transcode(
                &mut rmp_serde::decode::Deserializer::new(&mut self.sock),
                serde_json::value::Serializer,
            )?;

            println!("recv <-- {:>4}|{}", u32::from_be_bytes(msg_len), msg);

            let serde_json::Value::Array(msg) = msg else {
                anyhow::bail!("unexpected msg: expected array")
            };

            let res = self
                .process_packet(msg)
                .context("while processing packet")?;

            // jank in-band signalling to skip response
            if !res.is_null() {
                self.raw_send(res)?;
            }
        }
    }

    fn raw_send(&mut self, res: serde_json::Value) -> Result<()> {
        rmp_serde::encode::write_named(&mut self.out_buf, &res)?;

        {
            let v = rmpv::decode::read_value(&mut std::io::Cursor::new(self.out_buf.clone()))?;
            println!("send --> {:>4}|{:#}", self.out_buf.len(), v);
        }

        self.sock
            .write_all(&(self.out_buf.len() as u32).to_be_bytes())?;
        self.sock.write_all(&self.out_buf)?;
        self.out_buf.clear();

        Ok(())
    }

    fn handle_auth(
        &mut self,
        id: WampMsgCode,
        msg: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let timestamp = chrono::Local::now().format("%FT%T").to_string();

        let res = match id {
            WampMsgCode::Hello => {
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
                    WampMsgCode::Challenge as u64, // comment because of rustfmt
                    "wampcra",
                    { "challenge": challenge }
                ])
            }
            WampMsgCode::Authenticate => {
                // don't actually verify the challenge lmao
                let wamp_session_id = 1; // supposed to be random

                self.is_authed = true;

                json!([
                    WampMsgCode::Welcome as u64,
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
            code => anyhow::bail!("unexpected pre-auth WAMP message code: {code:?}"),
        };

        Ok(res)
    }

    fn process_packet(&mut self, msg: Vec<serde_json::Value>) -> Result<serde_json::Value> {
        let id = (msg[0].as_u64()).context(format!("unexpected id: {}", msg[0]))?;
        let id = WampMsgCode::try_from(id).context(format!("invalid WAMP message code: {}", id))?;

        if !self.is_authed {
            return self.handle_auth(id, msg);
        }

        let req_id = (msg[1].as_u64()).context(format!("unexpected req_id: {}", msg[1]))?;

        let res = match id {
            WampMsgCode::Subscribe => {
                if msg.len() != 4 {
                    anyhow::bail!("invalid SUBSCRIBE")
                }

                let (options, topic) = match (msg[2].as_object(), msg[3].as_str()) {
                    (None, _) => anyhow::bail!("invalid options: {}", msg[2]),
                    (_, None) => anyhow::bail!("invalid type: {}", msg[3]),
                    (Some(options), Some(topic)) => (options, topic),
                };

                {
                    println!(
                        "[{:>8}] SUBSCRIBE {} ({})",
                        req_id,
                        topic,
                        options
                            .iter()
                            .map(|(key, value)| format!("{key}={value}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }

                json!([
                    WampMsgCode::Subscribed as u64,
                    req_id,
                    self.handle_subscribe(options, topic)?
                ])
            }
            WampMsgCode::Call => {
                if msg.len() < 4 || msg.len() > 6 {
                    anyhow::bail!("invalid CALL")
                }

                let default_args = serde_json::Value::Array(Vec::new());
                let default_kwargs = serde_json::Value::Object(serde_json::Map::new());

                let proc = msg.get(3).unwrap();
                let args = msg.get(4).unwrap_or(&default_args);
                let kwargs = msg.get(5).unwrap_or(&default_kwargs);

                let (proc, args, kwargs) =
                    match (proc.as_str(), args.as_array(), kwargs.as_object()) {
                        (None, _, _) => anyhow::bail!("invalid proc: {}", msg[3]),
                        (_, None, _) => anyhow::bail!("invalid args: {}", msg[4]),
                        (_, _, None) => anyhow::bail!("invalid kwargs: {}", msg[5]),
                        (Some(proc), Some(args), Some(kwargs)) => (proc, args, kwargs),
                    };

                if !matches!(
                    proc,
                    "com.spotify.superbird.pitstop.log"
                        | "com.spotify.superbird.instrumentation.request"
                        | "com.spotify.superbird.instrumentation.log"
                ) {
                    println!(
                        "[{:>8}] CALL {}|{}|{}",
                        req_id,
                        proc,
                        args.iter()
                            .map(|arg| format!("{arg}"))
                            .collect::<Vec<_>>()
                            .join(", "),
                        kwargs
                            .iter()
                            .map(|(key, value)| format!("{key}={value}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }

                // jank jank jank
                match self.handle_rpc(proc, args, kwargs)? {
                    nil @ serde_json::Value::Null => nil,
                    res => json!([WampMsgCode::Result as u64, req_id, {}, res]),
                }
            }
            code => anyhow::bail!("unexpected WAMP message code: {code:?}"),
        };

        Ok(res)
    }

    fn handle_rpc(
        &self,
        proc: &str,
        args: &[serde_json::Value],
        kwargs: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let res = match proc {
            "com.spotify.superbird.pitstop.log"
            | "com.spotify.superbird.instrumentation.request"
            | "com.spotify.superbird.instrumentation.log" => json!(null),
            "com.spotify.superbird.ota.check_for_updates" => {
                json!({
                    "result": []
                })
            }
            "com.spotify.superbird.permissions" => {
                json!({ "can_use_superbird": true })
            }
            "com.spotify.superbird.register_device" => json!({}),
            _ => anyhow::bail!("unknown RPC: {proc}"),
        };

        Ok(res)
    }

    fn handle_subscribe(
        &mut self,
        options: &serde_json::Map<String, serde_json::Value>,
        topic: &str,
    ) -> Result<usize> {
        if !options.is_empty() {
            anyhow::bail!("unknown options")
        }

        let sub_id = self.next_sub_id;
        self.next_sub_id += 1;
        self.subs.entry(topic.to_owned()).or_default().push(sub_id);
        Ok(sub_id)
    }

    fn publish(&mut self, topic: &str, details: serde_json::Value, pub_id: usize) -> Result<()> {
        let subs = self
            .subs
            .get("topic")
            .context(format!("unknown topic: {}", topic))?
            .clone(); // bleh clone

        for sub_id in subs {
            self.raw_send(json!([
                WampMsgCode::Event as u64,
                sub_id,
                pub_id,
                {},
                [],
                details
            ]))?;
        }

        Ok(())
    }
}

// https://wamp-proto.org/wamp_bp_latest_ietf.html#name-message-codes-and-direction
// 4, 5, 49, 69 https://wamp-proto.org/wamp_latest_ietf.html#name-additional-messages
#[derive(Debug, num_enum::TryFromPrimitive)]
#[repr(u64)]
#[allow(unused)]
enum WampMsgCode {
    Hello = 1,
    Welcome = 2,
    Abort = 3,

    Challenge = 4,
    Authenticate = 5,

    Goodbye = 6,

    Error = 8,

    Publish = 16,
    Published = 17,

    Subscribe = 32,
    Subscribed = 33,
    Unsubscribe = 34,
    Unsubscribed = 35,
    Event = 36,

    Call = 48,
    Cancel = 49,
    Result = 50,

    Register = 64,
    Registered = 65,
    Unregister = 66,
    Unregistered = 67,
    Invocation = 68,
    Interrupt = 69,
    Yield = 70,
}
