//! One AirPlay control connection: request routing and per-sender state.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use playfair::FairPlay;
use plist::{Dictionary, Value};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::task::JoinHandle;

use crate::audio::{self, AudioCodec, AudioStream};
use crate::auth::Security;
use crate::crypto::{self, Pairing};
use crate::mdns::TxtRecords;
use crate::rtsp::{self, Request, Response};
use crate::{Event, Shared, consts, mirror, ntp};

const STREAM_AUDIO: u64 = 96;
const STREAM_MIRROR: u64 = 110;

pub(crate) async fn serve(stream: TcpStream, peer: SocketAddr, shared: Arc<Shared>) {
    let _ = stream.set_nodelay(true);
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut session = Session::new(shared, peer);

    loop {
        let request = match rtsp::read_request(&mut reader).await {
            Ok(Some(request)) => request,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("{peer}: {e}");
                break;
            }
        };
        tracing::debug!("{peer}: {} {} ({} bytes)", request.method, request.uri, request.body.len());

        let mut response = session.handle(&request).await;
        let cseq = request.header("CSeq");
        if cseq.is_some() && request.method != "RECORD" {
            response = response.header("Audio-Jack-Status", "connected; type=digital");
        }
        if response.status != 200 {
            tracing::warn!("{peer}: {} {} -> {}", request.method, request.uri, response.status);
        }
        if let Err(e) = write.write_all(&response.encode(&request.protocol, cseq)).await {
            tracing::warn!("{peer}: write failed: {e}");
            break;
        }
    }
}

struct Session {
    shared: Arc<Shared>,
    peer: SocketAddr,
    /// When the sender connected, to measure how long setting up takes.
    connected: Instant,
    fairplay: FairPlay,
    pairing: Pairing,
    /// Passed the receiver's access control (always, without security).
    authorized: bool,
    aes_key: Option<[u8; 16]>,
    aes_iv: Option<[u8; 16]>,
    announced: bool,
    timing: Option<JoinHandle<()>>,
    mirror: Option<JoinHandle<()>>,
    audio: Vec<JoinHandle<()>>,
}

impl Session {
    fn new(shared: Arc<Shared>, peer: SocketAddr) -> Self {
        Self {
            authorized: !shared.security.required(),
            shared,
            peer,
            connected: Instant::now(),
            fairplay: FairPlay::new(),
            pairing: Pairing::default(),
            aes_key: None,
            aes_iv: None,
            announced: false,
            timing: None,
            mirror: None,
            audio: Vec::new(),
        }
    }

    async fn handle(&mut self, req: &Request) -> Response {
        // Streams are set up and controlled only after access was granted.
        if !self.authorized && matches!(req.method.as_str(), "RECORD" | "FLUSH" | "GET_PARAMETER" | "SET_PARAMETER") {
            return Response::status(470, "Connection Authorization Required");
        }
        match (req.method.as_str(), req.uri.as_str()) {
            ("GET", "/info") => self.info(req),
            ("POST", "/pair-setup") => match self.pairing.setup(&self.shared.identity, &req.body) {
                Ok(pk) => Response::ok().body("application/octet-stream", pk.to_vec()),
                Err(e) => bad_request(e),
            },
            ("POST", "/pair-verify") => self.pair_verify(&req.body),
            ("POST", "/fp-setup") => self.fp_setup(&req.body),
            ("POST", "/feedback") | ("POST", "/audioMode") => Response::ok(),
            ("OPTIONS", _) => Response::ok().header(
                "Public",
                "SETUP, RECORD, FLUSH, TEARDOWN, OPTIONS, GET_PARAMETER, SET_PARAMETER",
            ),
            ("SETUP", _) => self.setup(req).await,
            ("GET_PARAMETER", _) => self.get_parameter(req),
            ("SET_PARAMETER", _) => self.set_parameter(req),
            ("FLUSH", _) => Response::ok(),
            ("RECORD", _) => Response::ok()
                .header("Audio-Latency", "0")
                .header("Audio-Jack-Status", "connected; type=analog"),
            ("TEARDOWN", _) => self.teardown(req),
            ("POST", uri) => {
                tracing::info!("{}: unhandled POST {uri}", self.peer);
                Response::ok()
            }
            (method, uri) => {
                tracing::info!("{}: unsupported {method} {uri}", self.peer);
                Response::status(501, "Not Implemented")
            }
        }
    }

    fn info(&self, req: &Request) -> Response {
        let shared = &self.shared;
        let mut dict = Dictionary::new();

        // Senders first ask only for one of our TXT records.
        let qualifier = if req.is_binary_plist() {
            parse_dict(&req.body)
                .and_then(|d| d.get("qualifier")?.as_array()?.first()?.as_string().map(str::to_owned))
        } else {
            None
        };
        let wants = |name: &str| match &qualifier {
            Some(q) => q == name,
            // Bluetooth LE discovery asks without CSeq, naming the record in the URL.
            None => req.header("CSeq").is_none() && req.uri.contains(name),
        };
        if wants("txtAirPlay") {
            dict.insert("txtAirPlay".into(), Value::Data(TxtRecords::encode(&shared.txt.airplay)));
        }
        if wants("txtRAOP") {
            dict.insert("txtRAOP".into(), Value::Data(TxtRecords::encode(&shared.txt.raop)));
        }
        if req.header("Content-Type").is_some() {
            return Response::ok().plist(Value::Dictionary(dict));
        }

        let device_id = shared.identity.device_id_string();
        dict.insert("deviceID".into(), Value::String(device_id.clone()));
        dict.insert("macAddress".into(), Value::String(device_id));
        dict.insert("pk".into(), Value::Data(shared.identity.public_key().to_vec()));
        dict.insert("features".into(), uint(shared.features));
        dict.insert("name".into(), Value::String(shared.name.clone()));
        dict.insert("pi".into(), Value::String(consts::PAIRING_IDENTITY.into()));
        dict.insert("vv".into(), uint(2));
        dict.insert("statusFlags".into(), uint(68));
        dict.insert("keepAliveLowPower".into(), Value::Boolean(true));
        dict.insert("sourceVersion".into(), Value::String(consts::SOURCE_VERSION.into()));
        dict.insert("keepAliveSendStatsAsBody".into(), Value::Boolean(true));
        dict.insert("model".into(), Value::String(consts::MODEL.into()));
        if req.header("CSeq").is_none() {
            return Response::ok().plist(Value::Dictionary(dict));
        }

        dict.insert("initialVolume".into(), Value::Real(0.0));
        let audio_entry = |kind: u64, latency: bool| {
            let mut d = Dictionary::new();
            d.insert("type".into(), uint(kind));
            d.insert("audioType".into(), Value::String("default".into()));
            if latency {
                d.insert("inputLatencyMicros".into(), uint(0));
                d.insert("outputLatencyMicros".into(), Value::Boolean(false));
            } else {
                d.insert("audioInputFormats".into(), uint(0x3FF_FFFC));
                d.insert("audioOutputFormats".into(), uint(0x3FF_FFFC));
            }
            Value::Dictionary(d)
        };
        dict.insert("audioLatencies".into(), Value::Array(vec![audio_entry(100, true), audio_entry(101, true)]));
        dict.insert("audioFormats".into(), Value::Array(vec![audio_entry(100, false), audio_entry(101, false)]));

        let display = &shared.display;
        let mut d = Dictionary::new();
        d.insert("uuid".into(), Value::String(consts::DISPLAY_UUID.into()));
        d.insert("widthPhysical".into(), uint(0));
        d.insert("heightPhysical".into(), uint(0));
        d.insert("width".into(), uint(display.width.into()));
        d.insert("height".into(), uint(display.height.into()));
        d.insert("widthPixels".into(), uint(display.width.into()));
        d.insert("heightPixels".into(), uint(display.height.into()));
        d.insert("rotation".into(), Value::Boolean(false));
        d.insert("refreshRate".into(), Value::Real(1.0 / f64::from(display.refresh_rate)));
        d.insert("maxFPS".into(), uint(display.max_fps.into()));
        d.insert("overscanned".into(), Value::Boolean(false));
        d.insert("features".into(), uint(14));
        dict.insert("displays".into(), Value::Array(vec![Value::Dictionary(d)]));

        Response::ok().plist(Value::Dictionary(dict))
    }

    fn pair_verify(&mut self, body: &[u8]) -> Response {
        match self.pairing.verify(&self.shared.identity, body) {
            Ok(reply) => {
                if self.pairing.is_verified() {
                    tracing::info!("{}: pair-verify completed", self.peer);
                }
                Response::ok().body("application/octet-stream", reply)
            }
            Err(e) => {
                tracing::warn!("{}: {e}", self.peer);
                Response::status(470, "Connection Authorization Required")
            }
        }
    }

    /// Access control on the first SETUP: HTTP Digest with a fixed password
    /// or with the code shown on screen. A device that typed the code once
    /// is remembered, and its iPhone answers with that code by itself.
    fn authorize(&mut self, req: &Request, root: &Dictionary) -> Result<(), Response> {
        let shared = self.shared.clone();
        let device = root.get("deviceID").and_then(Value::as_string).unwrap_or_default();
        let answer = req.header("Authorization");
        let with_code = matches!(shared.security, Security::Code | Security::CodeEveryTime);
        let remembered = match shared.security {
            Security::Code => shared.registry.lock().unwrap().code(device).map(str::to_owned),
            _ => None,
        };
        let password = match &shared.security {
            Security::None => return Ok(()),
            Security::Password(password) => password.clone(),
            Security::Code | Security::CodeEveryTime => match &remembered {
                Some(code) => code.clone(),
                None => {
                    let code = shared.auth.lock().unwrap().code_for(device, answer.is_none());
                    if answer.is_none() {
                        let _ = shared.events.send(Event::CodeShown { code: code.clone() });
                    }
                    code
                }
            },
        };

        let mut auth = shared.auth.lock().unwrap();
        if let Some(answer) = answer {
            if auth.check(answer, &req.method, &password) {
                self.authorized = true;
                if with_code && remembered.is_none() {
                    auth.clear_code();
                    let _ = shared.events.send(Event::CodeHidden);
                    if shared.security == Security::Code
                        && let Err(e) = shared.registry.lock().unwrap().add(device, &password)
                    {
                        tracing::warn!("cannot remember the device: {e}");
                    }
                }
                return Ok(());
            }
            tracing::warn!("{}: wrong password or code", self.peer);
            // The device no longer knows its code: it will get a new one.
            if remembered.is_some()
                && let Err(e) = shared.registry.lock().unwrap().remove(device)
            {
                tracing::warn!("cannot forget the device: {e}");
            }
        }
        Err(Response::status(401, "Unauthorized").header("WWW-Authenticate", auth.challenge()))
    }

    fn fp_setup(&mut self, body: &[u8]) -> Response {
        let reply = if let Ok(request) = <&[u8; 16]>::try_from(body) {
            self.fairplay.setup(request).map(|r| r.to_vec())
        } else if let Ok(request) = <&[u8; 164]>::try_from(body) {
            self.fairplay.handshake(request).map(|r| r.to_vec())
        } else {
            return bad_request(format!("fp-setup body of {} bytes", body.len()));
        };
        match reply {
            Ok(reply) => Response::ok().body("application/octet-stream", reply),
            Err(e) => {
                tracing::warn!("{}: {e}", self.peer);
                Response::status(501, "Not Implemented")
            }
        }
    }

    async fn setup(&mut self, req: &Request) -> Response {
        let Some(root) = parse_dict(&req.body) else {
            return bad_request("SETUP body is not a property list");
        };
        if !self.authorized
            && let Err(denied) = self.authorize(req, &root)
        {
            return denied;
        }
        let mut res = Dictionary::new();

        if let (Some(ekey), Some(eiv)) = (data(&root, "ekey"), data(&root, "eiv")) {
            let Ok(ekey) = <&[u8; 72]>::try_from(ekey) else {
                return bad_request(format!("ekey of {} bytes", ekey.len()));
            };
            let Ok(eiv) = <[u8; 16]>::try_from(eiv) else {
                return bad_request(format!("eiv of {} bytes", eiv.len()));
            };
            self.aes_iv = Some(eiv);
            let mut key = match self.fairplay.decrypt(ekey) {
                Ok(key) => key,
                Err(e) => return bad_request(e),
            };
            if let Some(secret) = self.pairing.ecdh_secret() {
                key = crypto::bind_key_to_pairing(&key, secret);
            }
            self.aes_key = Some(key);

            if !self.announced {
                self.announced = true;
                let text = |k| root.get(k).and_then(Value::as_string).map(str::to_owned);
                tracing::info!(
                    "{}: session from {:?} ({:?}), User-Agent {:?}",
                    self.peer,
                    text("name"),
                    text("model"),
                    req.header("User-Agent")
                );
                let _ = self.shared.events.send(Event::ClientConnected {
                    peer: self.peer,
                    name: text("name"),
                    model: text("model"),
                });
            }

            let remote_port = root.get("timingPort").and_then(as_u64).unwrap_or(0) as u16;
            let mut timing_port = 0;
            if remote_port != 0 {
                match ntp::start(SocketAddr::new(self.peer.ip(), remote_port)).await {
                    Ok((port, task)) => {
                        timing_port = port;
                        replace_task(&mut self.timing, task);
                    }
                    Err(e) => tracing::warn!("{}: cannot start timing: {e}", self.peer),
                }
            }
            res.insert("timingPort".into(), uint(timing_port.into()));
            res.insert("eventPort".into(), uint(0));
        }

        if let Some(streams) = root.get("streams").and_then(Value::as_array) {
            let mut out = Vec::new();
            for stream in streams.iter().filter_map(Value::as_dictionary) {
                match stream.get("type").and_then(as_u64) {
                    Some(STREAM_MIRROR) => match self.start_mirror(stream).await {
                        Ok(port) => out.push(stream_reply(STREAM_MIRROR, port, None)),
                        Err(e) => return bad_request(e),
                    },
                    Some(STREAM_AUDIO) => match self.start_audio(stream).await {
                        Ok((data, control)) => out.push(stream_reply(STREAM_AUDIO, data, Some(control))),
                        Err(e) => return bad_request(e),
                    },
                    other => tracing::warn!("{}: SETUP of unknown stream type {other:?}", self.peer),
                }
            }
            res.insert("streams".into(), Value::Array(out));
        }

        Response::ok().plist(Value::Dictionary(res))
    }

    async fn start_mirror(&mut self, stream: &Dictionary) -> anyhow::Result<u16> {
        let key = self.aes_key.ok_or_else(|| anyhow::anyhow!("mirror SETUP before key exchange"))?;
        let id = stream
            .get("streamConnectionID")
            .and_then(as_u64)
            .ok_or_else(|| anyhow::anyhow!("mirror SETUP without streamConnectionID"))?;
        let listener = TcpListener::bind("0.0.0.0:0").await?;
        let port = listener.local_addr()?.port();
        let task = tokio::spawn(mirror::run(listener, crypto::mirror_cipher(&key, id), self.shared.events.clone()));
        replace_task(&mut self.mirror, task);
        tracing::info!(
            "{}: mirror stream {id} expected on port {port}, {} ms after connecting",
            self.peer,
            self.connected.elapsed().as_millis()
        );
        Ok(port)
    }

    async fn start_audio(&mut self, stream: &Dictionary) -> anyhow::Result<(u16, u16)> {
        let (Some(key), Some(iv)) = (self.aes_key, self.aes_iv) else {
            anyhow::bail!("audio SETUP before key exchange");
        };
        let ct = stream.get("ct").and_then(as_u64).unwrap_or(0);
        let codec = AudioCodec::from_compression_type(ct)
            .ok_or_else(|| anyhow::anyhow!("unknown audio compression type {ct}"))?;
        let data = UdpSocket::bind("0.0.0.0:0").await?;
        let control = UdpSocket::bind("0.0.0.0:0").await?;
        let ports = (data.local_addr()?.port(), control.local_addr()?.port());
        // Wait for the previous stream to finish stopping the sink, so it
        // cannot stop the new one.
        for task in self.audio.drain(..) {
            task.abort();
            let _ = task.await;
        }
        tracing::info!("{}: {codec:?} audio stream on port {}", self.peer, ports.0);
        let stream = AudioStream { data, control, key, iv, codec };
        self.audio.push(tokio::spawn(audio::run(stream, self.shared.audio.clone())));
        Ok(ports)
    }

    fn set_parameter(&self, req: &Request) -> Response {
        let body = String::from_utf8_lossy(&req.body);
        if let Some(db) = body.strip_prefix("volume:").and_then(|v| v.trim().parse::<f32>().ok()) {
            tracing::debug!("{}: volume {db} dB", self.peer);
            self.shared.audio.set_volume(audio::volume_to_gain(db));
        }
        Response::ok()
    }

    fn stop_audio(&mut self) {
        for task in self.audio.drain(..) {
            task.abort();
        }
    }

    fn teardown(&mut self, req: &Request) -> Response {
        let types: Vec<u64> = parse_dict(&req.body)
            .and_then(|d| {
                let streams = d.get("streams")?.as_array()?;
                Some(streams.iter().filter_map(|s| s.as_dictionary()?.get("type").and_then(as_u64)).collect())
            })
            .unwrap_or_default();
        tracing::info!("{}: TEARDOWN {types:?}", self.peer);

        if (types.is_empty() || types.contains(&STREAM_MIRROR))
            && let Some(task) = self.mirror.take() {
                task.abort();
            }
        if types.is_empty() || types.contains(&STREAM_AUDIO) {
            self.stop_audio();
        }
        if types.is_empty()
            && let Some(task) = self.timing.take() {
                task.abort();
            }
        Response::ok()
    }

    fn get_parameter(&self, req: &Request) -> Response {
        if req.body.starts_with(b"volume") {
            Response::ok().body("text/parameters", b"volume: 0.000000\r\n".to_vec())
        } else {
            Response::ok()
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        for task in self.mirror.take().into_iter().chain(self.timing.take()).chain(self.audio.drain(..)) {
            task.abort();
        }
        tracing::info!("{}: disconnected", self.peer);
        let _ = self.shared.events.send(Event::ClientDisconnected { peer: self.peer });
    }
}

fn replace_task(slot: &mut Option<JoinHandle<()>>, task: JoinHandle<()>) {
    if let Some(old) = slot.replace(task) {
        old.abort();
    }
}

fn bad_request(reason: impl std::fmt::Display) -> Response {
    tracing::warn!("bad request: {reason}");
    Response::status(400, "Bad Request")
}

fn parse_dict(body: &[u8]) -> Option<Dictionary> {
    Value::from_reader(std::io::Cursor::new(body)).ok()?.into_dictionary()
}

fn data<'a>(dict: &'a Dictionary, key: &str) -> Option<&'a [u8]> {
    dict.get(key).and_then(Value::as_data)
}

/// Integers may arrive signed or unsigned; libplist-style raw u64 either way.
fn as_u64(value: &Value) -> Option<u64> {
    value.as_unsigned_integer().or_else(|| value.as_signed_integer().map(|v| v as u64))
}

fn uint(value: u64) -> Value {
    Value::Integer(value.into())
}

fn stream_reply(kind: u64, data_port: u16, control_port: Option<u16>) -> Value {
    let mut d = Dictionary::new();
    d.insert("type".into(), uint(kind));
    d.insert("dataPort".into(), uint(data_port.into()));
    if let Some(port) = control_port {
        d.insert("controlPort".into(), uint(port.into()));
    }
    Value::Dictionary(d)
}
