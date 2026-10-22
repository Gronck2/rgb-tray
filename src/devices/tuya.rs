//! Tuya Wi-Fi bulbs over the local network: protocol 3.3 (0x55AA frames on TCP 6668, JSON
//! encrypted with AES-128-ECB under the bulb's local key). Bulbs are listed in
//! `%APPDATA%\rgbtray\tuya.toml` (`[[device]]` with `id`, `local_key`, `name`), fetched once from
//! the Tuya cloud. The LAN address is found by trying each open port 6668 on the local /24
//! network and kept there as `local_ip`. A bulb that doesn't answer is looked for again a few times.
//!
//! A bulb takes one local client at a time: while the Smart Life app on a phone holds it, the
//! connection is reset (22.10.2026). Breathing and color cycle are firmware scenes (DP 25),
//! since Wi-Fi is too slow for app-drawn frames.

use super::{Device, Effect, Rgb};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows_sys::Win32::Security::Cryptography::*;

const PORT: u16 = 6668;
const CONTROL: u32 = 7;
const DP_QUERY: u32 = 10;
/// Wi-Fi bulbs in power saving may take a while to answer.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(1500);
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);
/// After a bulb didn't answer, it is looked for again after these many seconds (each after the previous).
const RETRY_AFTER: [u64; 5] = [1, 3, 10, 30, 60];

// Data points of v2 bulbs (category "dj").
const DP_SWITCH: &str = "20";
const DP_MODE: &str = "21";
/// White brightness 10..=1000. Writing it switches the bulb to white.
const DP_BRIGHT: &str = "22";
/// White temperature 0 (warm) ..= 1000 (cold).
const DP_TEMP: &str = "23";
/// Color: hex "HHHHSSSSVVVV", hue 0..=360, saturation and value 0..=1000.
const DP_COLOUR: &str = "24";
/// Scene: hex scene number, then units of switch time, fade time, mode (2 = fade), h, s, v, bright, temp.
const DP_SCENE: &str = "25";

/// Fade time of one scene step at normal speed (firmware units, 0..=100).
const BREATHE_STEP: f32 = 20.0;
const CYCLE_STEP: f32 = 20.0;
/// Bottom of breathing, as the app-drawn breathing (8%).
const BREATHE_FLOOR: u32 = 8;

/// A bulb listed in tuya.toml.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    id: String,
    name: String,
    key: [u8; 16],
    ip: Option<Ipv4Addr>,
}

fn path() -> PathBuf {
    crate::config::Config::dir().join("tuya.toml")
}

fn parse(text: &str) -> Vec<Entry> {
    let Ok(table) = text.parse::<toml::Table>() else { return Vec::new() };
    let Some(list) = table.get("device").and_then(|d| d.as_array()) else { return Vec::new() };
    let field = |d: &toml::Value, k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    list.iter()
        .filter_map(|d| {
            let key: [u8; 16] = field(d, "local_key").as_bytes().try_into().ok()?;
            let id = field(d, "id");
            (!id.is_empty()).then(|| Entry { name: field(d, "name"), ip: field(d, "local_ip").parse().ok(), id, key })
        })
        .collect()
}

/// tuya.toml with a bulb's LAN address set.
fn with_ip(text: &str, id: &str, ip: Ipv4Addr) -> Option<String> {
    let mut table = text.parse::<toml::Table>().ok()?;
    let d = table.get_mut("device")?.as_array_mut()?.iter_mut().find(|d| d.get("id").and_then(|v| v.as_str()) == Some(id))?;
    d.as_table_mut()?.insert("local_ip".into(), toml::Value::String(ip.to_string()));
    toml::to_string(&table).ok()
}

fn entries() -> Vec<Entry> {
    std::fs::read_to_string(path()).map(|t| parse(&t)).unwrap_or_default()
}

fn device_id(e: &Entry) -> String {
    format!("tuya-{}", e.id)
}

// ---------------------------------------------------------------- protocol

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// AES-128-ECB with PKCS#7 padding (Windows CNG).
fn aes(key: &[u8; 16], data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    unsafe {
        let mut alg: BCRYPT_ALG_HANDLE = std::ptr::null_mut();
        if BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_AES_ALGORITHM, std::ptr::null(), 0) != 0 {
            return None;
        }
        let mode: Vec<u16> = "ChainingModeECB\0".encode_utf16().collect();
        let mut out = None;
        let mut hkey: BCRYPT_KEY_HANDLE = std::ptr::null_mut();
        if BCryptSetProperty(alg, BCRYPT_CHAINING_MODE, mode.as_ptr() as *const u8, (mode.len() * 2) as u32, 0) == 0
            && BCryptGenerateSymmetricKey(alg, &mut hkey, std::ptr::null_mut(), 0, key.as_ptr(), 16, 0) == 0
        {
            let mut buf = vec![0u8; data.len() + 16];
            let mut len = 0u32;
            let f = if encrypt { BCryptEncrypt } else { BCryptDecrypt };
            let null = std::ptr::null_mut();
            if f(hkey, data.as_ptr(), data.len() as u32, std::ptr::null(), null, 0, buf.as_mut_ptr(), buf.len() as u32, &mut len, BCRYPT_BLOCK_PADDING) == 0 {
                buf.truncate(len as usize);
                out = Some(buf);
            }
            BCryptDestroyKey(hkey);
        }
        BCryptCloseAlgorithmProvider(alg, 0);
        out
    }
}

/// A message to the bulb: prefix, sequence number, command, length, payload, CRC, suffix.
fn frame(seq: u32, cmd: u32, payload: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(payload.len() + 24);
    for word in [0x55AA, seq, cmd, payload.len() as u32 + 8] {
        m.extend(word.to_be_bytes());
    }
    m.extend(payload);
    m.extend(crc32(&m).to_be_bytes());
    m.extend(0xAA55u32.to_be_bytes());
    m
}

/// The JSON of a reply payload: optionally "3.3" + 12 bytes of header, then encrypted.
fn open_payload(key: &[u8; 16], payload: &[u8]) -> Option<serde_json::Value> {
    let data = if payload.starts_with(b"3.3") { payload.get(15..)? } else { payload };
    serde_json::from_slice(&aes(key, data, false)?).ok()
}

/// One message from the bulb: (command, payload). Replies carry a return code before the payload.
fn read_frame(s: &mut TcpStream) -> Result<(u32, Vec<u8>), String> {
    let mut head = [0u8; 16];
    s.read_exact(&mut head).map_err(|e| e.to_string())?;
    let word = |i: usize| u32::from_be_bytes(head[i..i + 4].try_into().unwrap());
    let len = word(12) as usize;
    if word(0) != 0x55AA || !(12..=4096).contains(&len) {
        return Err("bad reply".into());
    }
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).map_err(|e| e.to_string())?;
    let crc = u32::from_be_bytes(body[len - 8..len - 4].try_into().unwrap());
    if crc != crc32(&[&head[..], &body[..len - 8]].concat()) {
        return Err("bad reply checksum".into());
    }
    Ok((word(8), body[4..len - 8].to_vec()))
}

fn now() -> String {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs().to_string()
}

/// Sends one command and reads the bulb's answer (its JSON, if it sent one). Tried twice: a
/// bulb right after another connection (the port scan, the previous command) may reset it.
fn request(ip: Ipv4Addr, key: &[u8; 16], cmd: u32, json: &serde_json::Value) -> Result<Option<serde_json::Value>, String> {
    request_once(ip, key, cmd, json).or_else(|_| {
        std::thread::sleep(Duration::from_millis(300));
        request_once(ip, key, cmd, json)
    })
}

fn request_once(ip: Ipv4Addr, key: &[u8; 16], cmd: u32, json: &serde_json::Value) -> Result<Option<serde_json::Value>, String> {
    let mut s = TcpStream::connect_timeout(&SocketAddr::from((ip, PORT)), CONNECT_TIMEOUT).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(REPLY_TIMEOUT));
    let _ = s.set_write_timeout(Some(REPLY_TIMEOUT));
    let mut payload = Vec::new();
    // Commands carry a version header; queries don't.
    if cmd == CONTROL {
        payload.extend(b"3.3");
        payload.extend([0u8; 12]);
    }
    payload.extend(aes(key, json.to_string().as_bytes(), true).ok_or("encryption failed")?);
    s.write_all(&frame(1, cmd, &payload)).map_err(|e| e.to_string())?;
    let (_, reply) = read_frame(&mut s)?;
    if reply.is_empty() {
        return Ok(None);
    }
    open_payload(key, &reply).map(Some).ok_or_else(|| "the bulb doesn't take the local key".into())
}

// ---------------------------------------------------------------- lighting

/// Hue 0..360, saturation and value 0..=1000.
fn hsv(c: Rgb) -> (u32, u32, u32) {
    let (r, g, b) = (c.0 as f32, c.1 as f32, c.2 as f32);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    ((h.round() as u32) % 360, (s * 1000.0).round() as u32, (max / 255.0 * 1000.0).round() as u32)
}

/// Value scaled by a 0..=100 brightness, within the bulb's 10..=1000.
fn level(v: u32, brightness: u8) -> u32 {
    (v * brightness.min(100) as u32 / 100).clamp(10, 1000)
}

fn scene(num: u8, step: f32, speed: f32, units: &[(u32, u32, u32)]) -> String {
    let t = (step / speed).round().clamp(1.0, 100.0) as u32;
    let mut s = format!("{num:02x}");
    for &(h, sat, v) in units {
        s += &format!("{t:02x}{t:02x}02{h:04x}{sat:04x}{v:04x}00000000");
    }
    s
}

/// The data points that show an effect.
fn dps(effect: Effect, color: Rgb, brightness: u8, speed: f32) -> serde_json::Value {
    use serde_json::json;
    let (h, s, v) = hsv(color);
    if effect == Effect::Off || brightness == 0 || v == 0 {
        return json!({ DP_SWITCH: false });
    }
    match effect {
        Effect::Breathe => {
            let top = level(v, brightness);
            let units = [(h, s, top), (h, s, (top * BREATHE_FLOOR / 100).max(10))];
            json!({ DP_SWITCH: true, DP_MODE: "scene", DP_SCENE: scene(5, BREATHE_STEP, speed, &units) })
        }
        Effect::Cycle | Effect::Wave => {
            let units: Vec<_> = (0..6).map(|i| (i * 60, 1000, level(1000, brightness))).collect();
            json!({ DP_SWITCH: true, DP_MODE: "scene", DP_SCENE: scene(6, CYCLE_STEP, speed, &units) })
        }
        // White and near-white take the white LEDs: their tint picks the temperature.
        _ if s < 100 => {
            let temp = (500 + (color.2 as i32 - color.0 as i32) * 500 / 255).clamp(0, 1000);
            json!({ DP_SWITCH: true, DP_MODE: "white", DP_BRIGHT: level(v, brightness), DP_TEMP: temp })
        }
        _ => json!({ DP_SWITCH: true, DP_MODE: "colour", DP_COLOUR: format!("{h:04x}{s:04x}{:04x}", level(v, brightness)) }),
    }
}

pub struct Bulb {
    entry: Entry,
}

impl Device for Bulb {
    fn apply(&self, effect: Effect, color: Rgb, brightness: u8) -> Result<(), String> {
        let Some(ip) = self.entry.ip else {
            rescan();
            return Err(super::NO_LINK.into());
        };
        let id = &self.entry.id;
        let json = serde_json::json!({ "devId": id, "uid": id, "t": now(), "dps": dps(effect, color, brightness, super::speed()) });
        match request(ip, &self.entry.key, CONTROL, &json) {
            Ok(_) => Ok(()),
            Err(e) => {
                crate::log(&format!("{}: {ip}: {e}", device_id(&self.entry)));
                rescan();
                Err(super::NO_LINK.into())
            }
        }
    }
}

// ---------------------------------------------------------------- finding bulbs

/// The PC's address on the network that leads outside (no packet is sent).
fn local_ipv4() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) => Some(ip),
        _ => None,
    }
}

/// The bulb answers a status query at this address.
fn answers(ip: Ipv4Addr, e: &Entry) -> bool {
    let json = serde_json::json!({ "gwId": e.id, "devId": e.id, "uid": e.id, "t": now() });
    matches!(request(ip, &e.key, DP_QUERY, &json), Ok(Some(r)) if r["devId"] == e.id.as_str())
}

/// Addresses on the local /24 network with port 6668 open.
fn open_ports() -> Vec<Ipv4Addr> {
    let Some(me) = local_ipv4() else { return Vec::new() };
    let o = me.octets();
    std::thread::scope(|sc| {
        let probes: Vec<_> = (1..=254u8)
            .map(|i| Ipv4Addr::new(o[0], o[1], o[2], i))
            .filter(|&ip| ip != me)
            .map(|ip| sc.spawn(move || TcpStream::connect_timeout(&SocketAddr::from((ip, PORT)), CONNECT_TIMEOUT).is_ok().then_some(ip)))
            .collect();
        probes.into_iter().filter_map(|p| p.join().ok().flatten()).collect()
    })
}

static SCANNING: AtomicBool = AtomicBool::new(false);

/// Whether the bulb answers now: at its address, or found elsewhere on the network (saved).
fn reach(e: &Entry, open: &[Ipv4Addr]) -> bool {
    if e.ip.is_some_and(|ip| answers(ip, e)) {
        return true;
    }
    let Some(ip) = open.iter().copied().filter(|&ip| Some(ip) != e.ip).find(|&ip| answers(ip, e)) else { return false };
    crate::log(&format!("{}: found at {ip}", device_id(e)));
    if let Some(text) = std::fs::read_to_string(path()).ok().and_then(|t| with_ip(&t, &e.id, ip)) {
        let _ = std::fs::write(path(), text);
    }
    true
}

/// A bulb didn't answer: looks for the bulbs again in the background, a few times over about
/// two minutes (a Wi-Fi bulb may not answer for a moment, 22.10.2026, or move to another
/// address). When one is back, the lighting is applied again, so it gets what it missed.
fn rescan() {
    if SCANNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mut missing = entries().len();
        for (round, wait) in RETRY_AFTER.iter().enumerate() {
            std::thread::sleep(Duration::from_secs(*wait));
            let open = open_ports();
            let lost: Vec<Entry> = entries().into_iter().filter(|e| !reach(e, &open)).collect();
            if lost.len() < missing {
                crate::apply_all();
            }
            missing = lost.len();
            if lost.is_empty() {
                break;
            }
            if round + 1 == RETRY_AFTER.len() {
                for e in &lost {
                    crate::log(&format!("{}: not found on the network ({} open Tuya ports)", device_id(e), open.len()));
                }
            }
        }
        SCANNING.store(false, Ordering::SeqCst);
    });
}

/// How the app finds Tuya bulbs (see `devices::DRIVERS`): every bulb in tuya.toml is listed;
/// it is reached (or found) when the lighting is applied.
pub const DRIVER: super::Driver = super::Driver {
    discover: |d| {
        for e in entries() {
            let id = device_id(&e);
            super::set_name(&id, &e.name);
            d.found.push(super::Found { id, dev: Box::new(Bulb { entry: e }) });
        }
    },
    ..super::Driver::NONE
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_frame() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let f = frame(1, CONTROL, &[0xAB]);
        assert_eq!(&f[..17], &[0, 0, 0x55, 0xAA, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 9, 0xAB]);
        assert_eq!(&f[21..], &[0, 0, 0xAA, 0x55]);
        assert_eq!(u32::from_be_bytes(f[17..21].try_into().unwrap()), crc32(&f[..17]));
    }

    #[test]
    fn aes_ecb_round_trip() {
        let key = *b"0123456789abcdef";
        let enc = aes(&key, br#"{"dps":{"20":true}}"#, true).unwrap();
        let hex: String = enc.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "6c43740537c46421392c83bfa222a5ee31836d84bd28cb4b20619eb84882ca2a");
        assert_eq!(open_payload(&key, &enc).unwrap()["dps"]["20"], true);
        let mut with_header = b"3.3".to_vec();
        with_header.extend([0u8; 12]);
        with_header.extend(&enc);
        assert!(open_payload(&key, &with_header).is_some());
        assert!(open_payload(b"fedcba9876543210", &enc).is_none());
    }

    #[test]
    fn colors_to_hsv() {
        assert_eq!(hsv(Rgb(255, 0, 0)), (0, 1000, 1000));
        assert_eq!(hsv(Rgb(0, 255, 0)), (120, 1000, 1000));
        assert_eq!(hsv(Rgb(0, 0, 128)), (240, 1000, 502));
        assert_eq!(hsv(Rgb(255, 0, 255)), (300, 1000, 1000));
        assert_eq!(hsv(Rgb(255, 255, 255)), (0, 0, 1000));
    }

    #[test]
    fn effects_to_data_points() {
        let red = Rgb(255, 0, 0);
        assert_eq!(dps(Effect::Off, red, 100, 1.0), serde_json::json!({"20": false}));
        assert_eq!(dps(Effect::Static, red, 0, 1.0), serde_json::json!({"20": false}));
        assert_eq!(dps(Effect::Static, red, 50, 1.0), serde_json::json!({"20": true, "21": "colour", "24": "000003e801f4"}));
        // White goes to the white LEDs, neutral; a bluish white is colder.
        assert_eq!(dps(Effect::Static, Rgb(255, 255, 255), 100, 1.0), serde_json::json!({"20": true, "21": "white", "22": 1000, "23": 500}));
        assert!(dps(Effect::Static, Rgb(235, 240, 255), 100, 1.0)["23"].as_i64().unwrap() > 500);
        // Breathing fades between the color and 8% of it.
        let b = dps(Effect::Breathe, Rgb(0, 255, 0), 100, 1.0);
        assert_eq!(b["25"], "05141402007803e803e800000000141402007803e8005000000000");
        // Faster speed, shorter steps; never 0.
        assert!(dps(Effect::Cycle, red, 100, 4.0)["25"].as_str().unwrap().starts_with("060505"));
        assert!(dps(Effect::Cycle, red, 100, 1000.0)["25"].as_str().unwrap().starts_with("060101"));
        assert_eq!(dps(Effect::Cycle, red, 100, 1.0)["25"].as_str().unwrap().len(), 2 + 6 * 26);
    }

    /// `cargo test --release -- --ignored network`: the bulbs in the real tuya.toml are found on
    /// the network and answer a status query (nothing is switched).
    #[test]
    #[ignore]
    fn network_tuya_bulbs_answer() {
        let open = open_ports();
        for e in entries() {
            let ip = open.iter().copied().find(|&ip| answers(ip, &e));
            println!("{}: {ip:?} (open: {open:?})", device_id(&e));
            assert!(ip.is_some());
        }
    }

    #[test]
    fn bulb_list() {
        let text = "endpoint = \"x\"\n\n[[device]]\nname = \"Лампа\"\nid = \"abc\"\nlocal_key = \"0123456789abcdef\"\n\n[[device]]\nid = \"short\"\nlocal_key = \"123\"\n";
        let list = parse(text);
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].name.as_str(), list[0].ip), ("Лампа", None));
        let text = with_ip(text, "abc", Ipv4Addr::new(192, 168, 1, 5)).unwrap();
        assert_eq!(parse(&text)[0].ip, Some(Ipv4Addr::new(192, 168, 1, 5)));
        assert!(text.contains("endpoint"));
        assert!(parse("not toml [").is_empty());
    }
}
