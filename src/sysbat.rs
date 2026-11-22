//! Batteries of devices the app does not drive but Windows knows: Bluetooth headphones, mice, keyboards and pads
//! for which Windows holds a level (the one Settings > Bluetooth & devices shows, `DEVPKEY_Bluetooth_Battery`).
//! They are listed on the battery overlay and raise the low-battery alert.
//!
//! The Bluetooth part only reads PnP properties of the device nodes, which are a cache in the Windows store: no device
//! handle is opened and nothing is sent to a device (no GATT, HFP or vendor protocol), so nothing is woken.
//! The one exception: Logitech mice and headsets on a LIGHTSPEED receiver, a cable or a headset dongle are asked their
//! battery over HID++ (`logitech::other_batteries`, reads only). That runs only while the user is active
//! (`IDLE_SKIP`) and `overlay_system` is on, and never touches a slot the app's own Logitech driver drives.
//! System devices are kept apart from `PRESENT`, `STATUS` and `config.devices`: the panel, hotkeys and settings never see them.
//!
//! The level sits on a service node (e.g. the Hands-Free one), the connection state on the device node
//! (`DEV_<address>`): nodes are joined by the Bluetooth address in the instance id.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// What a device looks like on the overlay.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SysClass {
    Headphones,
    Headset,
    Speaker,
    Mouse,
    Keyboard,
    Gamepad,
    Phone,
    Other,
}

impl SysClass {
    /// For `RGBTRAY_DEMO_SYSTEM`.
    fn parse(name: &str) -> SysClass {
        match name.trim().to_ascii_lowercase().as_str() {
            "headphones" => SysClass::Headphones,
            "headset" => SysClass::Headset,
            "speaker" => SysClass::Speaker,
            "mouse" => SysClass::Mouse,
            "keyboard" => SysClass::Keyboard,
            "gamepad" => SysClass::Gamepad,
            "phone" => SysClass::Phone,
            _ => SysClass::Other,
        }
    }
}

/// A connected device with a battery level, as Windows reports it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SysDevice {
    /// `system-<hash of the Bluetooth address>`: only the key of the alert state and the sort order, never saved.
    pub id: String,
    pub name: String,
    pub class: SysClass,
    pub percent: u8,
}

/// The devices found by the last read.
static SYSTEM: Mutex<Vec<SysDevice>> = Mutex::new(Vec::new());
/// The last Logitech HID++ rows (`logitech::other_batteries`), kept while a read is skipped (the user is away).
static HIDPP: Mutex<Vec<SysDevice>> = Mutex::new(Vec::new());

/// The class of a device from its Bluetooth Class of Device (classic devices only; none for most LE ones).
fn class_of(cod: Option<u32>) -> SysClass {
    let Some(cod) = cod else { return SysClass::Other };
    let (major, minor) = ((cod >> 8) & 0x1F, (cod >> 2) & 0x3F);
    match major {
        0x02 => SysClass::Phone,
        0x04 => match minor {
            0x02 | 0x04 => SysClass::Headset,
            // Loudspeaker, portable audio, hi-fi.
            0x05 | 0x07 | 0x0A => SysClass::Speaker,
            _ => SysClass::Headphones,
        },
        // Peripheral: bit 4 of the minor is a keyboard (a combo is one too), bit 5 a pointing device.
        0x05 if minor & 0x10 != 0 => SysClass::Keyboard,
        0x05 if minor & 0x20 != 0 => SysClass::Mouse,
        0x05 if matches!(minor & 0x0F, 1 | 2) => SysClass::Gamepad,
        _ => SysClass::Other,
    }
}

/// The glyph of Segoe Fluent Icons (also in Segoe MDL2 Assets) for the overlay.
pub fn glyph(class: SysClass) -> &'static str {
    match class {
        SysClass::Headphones => "\u{E7F6}",
        SysClass::Headset => "\u{E95B}",
        SysClass::Speaker => "\u{E7F5}",
        SysClass::Mouse => "\u{E962}",
        SysClass::Keyboard => "\u{E765}",
        SysClass::Gamepad => "\u{E7FC}",
        SysClass::Phone => "\u{E8EA}",
        SysClass::Other => "\u{E702}",
    }
}

fn is_hex_address(s: &str) -> bool {
    s.len() == 12 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The Bluetooth address (12 hex digits, upper case) in an instance id: `BTHENUM\DEV_58186221E9F9\...`,
/// `BTHLE\DEV_...` or the last part of a service node (`BTHENUM\{0000111E-...}_VID&...\7&2A&0&58186221E9F9_C00000000`).
fn address_of(instance: &str) -> Option<String> {
    let upper = instance.to_ascii_uppercase();
    if let Some(a) = upper.find("DEV_").and_then(|i| upper.get(i + 4..i + 16)).filter(|a| is_hex_address(a)) {
        return Some(a.to_string());
    }
    let last = upper.rsplit('\\').next()?;
    last.split(['&', '_']).rev().find(|t| is_hex_address(t)).map(str::to_string)
}

/// `system-<8 hex>`: FNV-1a of the address (of the whole instance id when it has none), case-insensitive. The address
/// is the same for the classic and the LE node of one device and over reconnects.
fn id_for(instance: &str) -> String {
    let key = address_of(instance).unwrap_or_else(|| instance.to_ascii_uppercase());
    let mut h: u32 = 0x811C9DC5;
    for b in key.bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    format!("system-{h:08X}")
}

/// What was read from one Bluetooth device node.
#[derive(Clone, Debug, Default)]
struct Node {
    instance: String,
    name: Option<String>,
    cod: Option<u32>,
    connected: Option<bool>,
    battery: Option<u8>,
    /// Whether the node is started now: a connected device may keep its level on a service node that is not (A2DP only, no Hands-Free).
    present: Option<bool>,
}

/// All the nodes of one device, joined by address.
#[derive(Clone, Debug, PartialEq)]
struct Joined {
    id: String,
    name: String,
    class: SysClass,
    connected: bool,
    percent: Option<u8>,
}

/// "WH-1000XM6 Hands-Free AG" names a service; the device's own name is without the suffix.
fn plain_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for suffix in [" hands-free ag", " hands-free"] {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].trim_end().to_string();
        }
    }
    name.to_string()
}

fn join(nodes: &[Node]) -> Vec<Joined> {
    let mut groups: Vec<(String, Vec<&Node>)> = Vec::new();
    for n in nodes {
        let id = id_for(&n.instance);
        match groups.iter_mut().find(|(g, _)| *g == id) {
            Some((_, v)) => v.push(n),
            None => groups.push((id, vec![n])),
        }
    }
    groups
        .into_iter()
        .map(|(id, nodes)| {
            // The device node (`DEV_...`) carries the device's own name; a service node only the name of the service.
            let own = nodes.iter().find(|n| n.instance.to_ascii_uppercase().contains("\\DEV_")).and_then(|n| n.name.clone());
            let name = own.or_else(|| nodes.iter().find_map(|n| n.name.as_deref().map(plain_name))).filter(|n| !n.trim().is_empty());
            Joined {
                id,
                name: name.unwrap_or_else(|| "Bluetooth device".into()),
                class: class_of(nodes.iter().find_map(|n| n.cod)),
                // Only a present node knows: a node left behind (radio off, dongle out) keeps its last "connected".
                connected: nodes.iter().any(|n| n.present != Some(false) && n.connected == Some(true)),
                // A level from a present node first, else from any (not present) one.
                percent: nodes.iter().filter(|n| n.present != Some(false)).find_map(|n| n.battery).or_else(|| nodes.iter().find_map(|n| n.battery)),
            }
        })
        .collect()
}

/// Connected devices with a level: Windows keeps the old level of a disconnected one.
fn devices(joined: Vec<Joined>) -> Vec<SysDevice> {
    joined.into_iter().filter(|j| j.connected).filter_map(|j| Some(SysDevice { id: j.id, name: j.name, class: j.class, percent: j.percent? })).collect()
}

/// The same product as one of the app's own devices: the names are equal, or one holds the other (4 characters or more).
fn is_duplicate(name: &str, app_names: &[String]) -> bool {
    let a = name.trim().to_lowercase();
    app_names.iter().any(|b| {
        let b = b.trim().to_lowercase();
        let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
        !a.is_empty() && !b.is_empty() && (a == b || (short.chars().count() >= 4 && long.contains(short.as_str())))
    })
}

/// What the overlay and the alert use: nothing when the setting is off, the app's own devices left out, by name then id.
fn visible(found: &[SysDevice], on: bool, app_names: &[String]) -> Vec<SysDevice> {
    if !on {
        return Vec::new();
    }
    let mut v: Vec<SysDevice> = found.iter().filter(|d| !is_duplicate(&d.name, app_names)).cloned().collect();
    v.sort_by_cached_key(|d| (d.name.to_lowercase(), d.id.clone()));
    v
}

/// The system devices to show now (sorted), or none when the user turned them off.
pub fn listed() -> Vec<SysDevice> {
    // Ids first: active_ids() locks the config too.
    let ids = crate::active_ids();
    let cfg = crate::config();
    let mut app_names = Vec::new();
    for id in &ids {
        let kind = crate::devices::kind_of(id);
        app_names.extend([cfg.display_name(id), kind.names.0.to_string(), kind.names.1.to_string()]);
    }
    let on = cfg.overlay_system;
    drop(cfg);
    visible(&SYSTEM.lock().unwrap(), on, &app_names)
}

/// `RGBTRAY_DEMO_SYSTEM=WH-1000XM6:headphones:72,Pad:gamepad:4`: made-up devices for snapshots.
fn parse_demo(spec: &str) -> Vec<SysDevice> {
    spec.split(',')
        .filter_map(|item| {
            let mut p = item.trim().split(':');
            let (name, class, percent) = (p.next()?.trim(), p.next()?, p.next()?.trim().parse::<u8>().ok()?);
            Some(SysDevice { id: id_for(name), name: name.to_string(), class: SysClass::parse(class), percent: percent.min(100) })
        })
        .collect()
}

pub fn demo(spec: &str) {
    *SYSTEM.lock().unwrap() = parse_demo(spec);
}

// ---------------------------------------------------------------- reading the properties

/// A value of a property buffer by its type; anything else reads as nothing.
fn parse_level(ty: u32, buf: &[u8]) -> Option<u8> {
    const BYTE: u32 = 3;
    (ty == BYTE && buf.len() == 1 && buf[0] <= 100).then(|| buf[0])
}

fn parse_bool(ty: u32, buf: &[u8]) -> Option<bool> {
    const BOOLEAN: u32 = 17;
    (ty == BOOLEAN && buf.len() == 1).then(|| buf[0] != 0)
}

fn parse_u32(ty: u32, buf: &[u8]) -> Option<u32> {
    const UINT32: u32 = 7;
    if ty != UINT32 {
        return None;
    }
    <[u8; 4]>::try_from(buf).ok().map(u32::from_le_bytes)
}

fn parse_string(ty: u32, buf: &[u8]) -> Option<String> {
    const STRING: u32 = 18;
    if ty != STRING {
        return None;
    }
    let w: Vec<u16> = buf.chunks(2).filter_map(|c| Some(u16::from_le_bytes(c.try_into().ok()?))).take_while(|&c| c != 0).collect();
    Some(String::from_utf16_lossy(&w)).filter(|s| !s.trim().is_empty())
}

/// The ids of the enumerators that could be read, and the errors of those that could not, joined (empty when none failed).
/// One failing enumerator does not hide the other's nodes, but its error is still reported.
fn merge_lists(lists: Vec<Result<Vec<String>, String>>) -> (Vec<String>, String) {
    let (ok, err): (Vec<_>, Vec<_>) = lists.into_iter().partition(|l| l.is_ok());
    (ok.into_iter().flatten().flatten().collect(), err.into_iter().filter_map(Result::err).collect::<Vec<_>>().join("; "))
}

mod win {
    use std::ptr::null_mut;

    use windows_sys::core::GUID;
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
    use windows_sys::Win32::Devices::Properties::DEVPROPKEY;

    use super::*;

    const fn key(guid: u128, pid: u32) -> DEVPROPKEY {
        DEVPROPKEY { fmtid: GUID::from_u128(guid), pid }
    }

    const BATTERY: DEVPROPKEY = key(0x104EA319_6EE2_4701_BD47_8DDBF425BBE5, 2);
    const IS_PRESENT: DEVPROPKEY = key(0x540B947E_8B40_45BC_A8A2_6A0B894CBDA2, 5);
    const CONNECTED: DEVPROPKEY = key(0x83DA6326_97A6_4088_9453_A1923F573B29, 15);
    const FRIENDLY_NAME: DEVPROPKEY = key(0xA45C254E_DF1C_4EFD_8020_67D146A850E0, 14);
    const NAME: DEVPROPKEY = key(0xB725F130_47EF_101A_A5F1_02608C9EEBAC, 10);
    const CLASS_OF_DEVICE: DEVPROPKEY = key(0x2BD67D8B_8BEB_48D5_87E0_6CDA3428040A, 10);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// The device nodes under one enumerator (`BTHENUM`, `BTHLE`), the not present ones too.
    unsafe fn instance_ids(enumerator: &str) -> Result<Vec<String>, String> {
        let filter = wide(enumerator);
        let flags = CM_GETIDLIST_FILTER_ENUMERATOR;
        // A node may appear between the size and the list (CR_BUFFER_SMALL): ask the size again.
        for _ in 0..4 {
            let mut len = 0u32;
            let r = CM_Get_Device_ID_List_SizeW(&mut len, filter.as_ptr(), flags);
            if r != CR_SUCCESS {
                return Err(format!("{enumerator}: CM_Get_Device_ID_List_Size failed ({r:#x})"));
            }
            let mut buf = vec![0u16; len as usize];
            match CM_Get_Device_ID_ListW(filter.as_ptr(), buf.as_mut_ptr(), len, flags) {
                CR_SUCCESS => return Ok(buf.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()),
                CR_BUFFER_SMALL => continue,
                r => return Err(format!("{enumerator}: CM_Get_Device_ID_List failed ({r:#x})")),
            }
        }
        Err(format!("{enumerator}: the device list kept changing"))
    }

    /// The type and bytes of a property of a node; `None` when it has none. Only a getter: nothing reaches the device.
    unsafe fn property(node: u32, key: &DEVPROPKEY) -> Option<(u32, Vec<u8>)> {
        let (mut ty, mut size) = (0u32, 0u32);
        // Asking with no buffer says how large it must be (CR_BUFFER_SMALL).
        if CM_Get_DevNode_PropertyW(node, key, &mut ty, null_mut(), &mut size, 0) != CR_BUFFER_SMALL || size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if CM_Get_DevNode_PropertyW(node, key, &mut ty, buf.as_mut_ptr(), &mut size, 0) != CR_SUCCESS {
            return None;
        }
        buf.truncate(size as usize);
        Some((ty, buf))
    }

    /// The nodes of the enumerators that could be read, and the errors of those that could not (empty when none failed).
    pub(super) fn read_nodes() -> (Vec<Node>, String) {
        let mut nodes = Vec::new();
        let lists: Vec<_> = ["BTHENUM", "BTHLE"].into_iter().map(|e| unsafe { instance_ids(e) }).collect();
        let (ids, errors) = merge_lists(lists);
        for instance in ids {
            let mut dn = 0u32;
            if unsafe { CM_Locate_DevNodeW(&mut dn, wide(&instance).as_ptr(), CM_LOCATE_DEVNODE_PHANTOM) } != CR_SUCCESS {
                continue;
            }
            let get = |k: &DEVPROPKEY| unsafe { property(dn, k) };
            nodes.push(Node {
                name: get(&FRIENDLY_NAME).and_then(|(t, b)| parse_string(t, &b)).or_else(|| get(&NAME).and_then(|(t, b)| parse_string(t, &b))),
                cod: get(&CLASS_OF_DEVICE).and_then(|(t, b)| parse_u32(t, &b)),
                connected: get(&CONNECTED).and_then(|(t, b)| parse_bool(t, &b)),
                battery: get(&BATTERY).and_then(|(t, b)| parse_level(t, &b)),
                present: get(&IS_PRESENT).and_then(|(t, b)| parse_bool(t, &b)),
                instance,
            });
        }
        (nodes, errors)
    }
}

/// Reads the system again; whether the list changed. An error (no Bluetooth, no access) is logged when its text
/// changes, not on every read; the nodes of an enumerator that did work are still used.
fn refresh() -> bool {
    static LAST_ERROR: Mutex<String> = Mutex::new(String::new());
    let (nodes, errors) = win::read_nodes();
    {
        let mut last = LAST_ERROR.lock().unwrap();
        if *last != errors {
            if !errors.is_empty() {
                crate::log(&format!("system batteries: {errors}"));
            }
            *last = errors;
        }
    }
    let mut found = devices(join(&nodes));
    // Wireless mice and headsets Windows has no level for: Logitech ones over HID++. That asks the devices, so only
    // while the user is active and wants system devices; else the last rows stay (or go, when the setting is off).
    let on = crate::config().overlay_system;
    let mut hidpp = HIDPP.lock().unwrap();
    if !on {
        hidpp.clear();
    } else if crate::idle_seconds() <= crate::IDLE_SKIP {
        // Serialised with applies and polls: no two requests to a receiver at once.
        let _guard = crate::APPLY_LOCK.lock().unwrap();
        *hidpp = crate::devices::logitech::other_batteries(crate::devices::logitech::SW_ID_OTHERS);
    }
    found.extend(hidpp.iter().cloned());
    drop(hidpp);
    let mut cur = SYSTEM.lock().unwrap();
    let changed = *cur != found;
    *cur = found;
    changed
}

/// Reads on a worker thread (never the UI thread) and tells the main window when the list changed.
pub fn refresh_async() {
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        if refresh() {
            crate::post(crate::WM_BATTERY);
        }
        BUSY.store(false, Ordering::SeqCst);
    });
}

/// Like `refresh_async`, but tells the main window whether or not the list changed (after the user turned the system devices on).
pub fn refresh_and_notify() {
    std::thread::spawn(|| {
        refresh();
        crate::post(crate::WM_BATTERY);
    });
}

/// `--system-batteries <out>`: every Bluetooth node with what Windows holds for it, and what the app makes of it.
/// It is for finding out why a device does not show up; it only reads properties (and Logitech batteries over HID++).
pub fn report() -> String {
    let (nodes, errors) = win::read_nodes();
    let yn = |v: Option<bool>| v.map_or("unknown", |b| if b { "yes" } else { "no" });
    let mut out = String::new();
    if !errors.is_empty() {
        out += &format!("Cannot read part of the Bluetooth nodes: {errors}\n");
    }
    out += &format!("Bluetooth device nodes: {}\n\n", nodes.len());
    for n in &nodes {
        out += &format!(
            "{}\n  name: {}\n  class of device: {}\n  present: {}\n  connected: {}\n  battery: {}\n  address: {}\n  id: {}\n",
            n.instance,
            n.name.as_deref().unwrap_or("none"),
            n.cod.map_or("none".into(), |c| format!("{c:#08x} -> {:?}", class_of(Some(c)))),
            yn(n.present),
            yn(n.connected),
            n.battery.map_or("none".into(), |b| format!("{b}%")),
            address_of(&n.instance).unwrap_or_else(|| "none".into()),
            id_for(&n.instance),
        );
    }
    out += "\nDevices (nodes joined by address):\n";
    for j in join(&nodes) {
        let shown = match (j.connected, j.percent) {
            (true, Some(p)) => format!("listed at {p}%"),
            (true, None) => "connected, no level: not listed".into(),
            (false, Some(p)) => format!("not connected (stale {p}%): hidden"),
            (false, None) => "not connected, no level: hidden".into(),
        };
        out += &format!("  {}  {}  {:?}  {shown}\n", j.id, j.name, j.class);
    }
    // Feature lookups and battery reads over HID++, nothing that changes a device.
    out += "\nLogitech HID++ devices the app doesn't light:\n";
    for d in crate::devices::logitech::other_batteries(crate::devices::logitech::SW_ID_REPORT) {
        out += &format!("  {}  {}  {:?}  listed at {}%\n", d.id, d.name, d.class, d.percent);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const XM6_DEVICE: &str = "BTHENUM\\DEV_58186221E9F9\\8&2A3B4C5D&0&BLUETOOTHDEVICE_58186221E9F9";
    const XM6_HFP: &str = "BTHENUM\\{0000111E-0000-1000-8000-00805F9B34FB}_VID&0002054C_PID&0DF3\\8&2A3B4C5D&0&58186221E9F9_C00000000";

    fn node(instance: &str, name: Option<&str>, cod: Option<u32>, connected: Option<bool>, battery: Option<u8>) -> Node {
        Node { instance: instance.into(), name: name.map(String::from), cod, connected, battery, present: Some(true) }
    }

    fn sys(name: &str, percent: u8) -> SysDevice {
        SysDevice { id: id_for(name), name: name.into(), class: SysClass::Other, percent }
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_failing_enumerator_is_skipped_but_reported() {
        let ok = Ok(names(&["a", "b"]));
        assert_eq!(merge_lists(vec![Err("x".into()), ok.clone()]), (names(&["a", "b"]), "x".into()));
        assert_eq!(merge_lists(vec![ok.clone(), Ok(names(&["c"]))]), (names(&["a", "b", "c"]), String::new()));
        assert_eq!(merge_lists(vec![Err("x".into()), Err("y".into())]), (Vec::new(), "x; y".into()));
    }

    #[test]
    fn class_follows_the_class_of_device() {
        assert_eq!(class_of(Some(0x240418)), SysClass::Headphones);
        assert_eq!(class_of(Some(0x240404)), SysClass::Headphones); // wearable headset
        assert_eq!(class_of(Some(0x240408)), SysClass::Headset); // hands-free
        assert_eq!(class_of(Some(0x240414)), SysClass::Speaker);
        assert_eq!(class_of(Some(0x002580)), SysClass::Mouse);
        assert_eq!(class_of(Some(0x002540)), SysClass::Keyboard);
        assert_eq!(class_of(Some(0x0025C0)), SysClass::Keyboard); // combo
        assert_eq!(class_of(Some(0x002508)), SysClass::Gamepad);
        assert_eq!(class_of(Some(0x5A020C)), SysClass::Phone);
        assert_eq!(class_of(None), SysClass::Other);
        assert_eq!(class_of(Some(0x000000)), SysClass::Other);
        assert_eq!(class_of(Some(0x00010C)), SysClass::Other); // computer
    }

    #[test]
    fn every_class_has_a_private_use_glyph() {
        use SysClass::*;
        for c in [Headphones, Headset, Speaker, Mouse, Keyboard, Gamepad, Phone, Other] {
            let g = glyph(c);
            assert_eq!(g.chars().count(), 1);
            assert!(('\u{E000}'..='\u{F8FF}').contains(&g.chars().next().unwrap()), "{c:?}");
        }
    }

    #[test]
    fn address_comes_from_the_device_and_the_service_nodes() {
        assert_eq!(address_of(XM6_DEVICE).as_deref(), Some("58186221E9F9"));
        assert_eq!(address_of(XM6_HFP).as_deref(), Some("58186221E9F9"));
        assert_eq!(address_of("BTHLE\\DEV_c9f8a2b1d3e4\\7&1A&0&c9f8a2b1d3e4").as_deref(), Some("C9F8A2B1D3E4"));
        assert_eq!(address_of("BTHENUM\\DEV_ZZ\\1&2&3"), None);
        assert_eq!(address_of("BTHENUM\\SOMETHING\\1&2&3"), None);
    }

    #[test]
    fn id_is_stable_case_insensitive_and_per_address() {
        let id = id_for(XM6_DEVICE);
        assert!(id.starts_with("system-") && id.len() == "system-".len() + 8);
        // The classic device node, its service node and an LE node of the same address agree.
        assert_eq!(id, id_for(XM6_HFP));
        assert_eq!(id, id_for("BTHLE\\DEV_58186221e9f9\\9&1&0&58186221e9f9"));
        assert_ne!(id, id_for("BTHENUM\\DEV_58186221E9FA\\8&2A3B4C5D&0&BLUETOOTHDEVICE_58186221E9FA"));
        // No address: the whole instance id, again case-insensitive.
        assert_eq!(id_for("BTHENUM\\Odd\\1&2"), id_for("bthenum\\odd\\1&2"));
        assert_ne!(id_for("BTHENUM\\Odd\\1&2"), id_for("BTHENUM\\Odd\\1&3"));
    }

    #[test]
    fn battery_and_connection_are_joined_by_address() {
        // As found on the owner's PC: the level on the Hands-Free node, the connection and name on the device node.
        let nodes = [
            node(XM6_HFP, Some("WH-1000XM6 Hands-Free AG"), None, None, Some(94)),
            node(XM6_DEVICE, Some("WH-1000XM6"), Some(0x240404), Some(true), None),
            node("BTHLE\\DEV_C9F8A2B1D3E4\\7&1A&0&c9f8a2b1d3e4", Some("Xbox Wireless Controller"), None, Some(false), Some(80)),
        ];
        let j = join(&nodes);
        assert_eq!(j.len(), 2);
        assert_eq!(j[0], Joined { id: id_for(XM6_DEVICE), name: "WH-1000XM6".into(), class: SysClass::Headphones, connected: true, percent: Some(94) });
        let list = devices(j);
        // The pad keeps its stale level in Windows but is not connected: hidden.
        assert_eq!(list, [SysDevice { id: id_for(XM6_DEVICE), name: "WH-1000XM6".into(), class: SysClass::Headphones, percent: 94 }]);
    }

    #[test]
    fn name_falls_back_to_the_service_name_without_its_suffix() {
        let j = join(&[node(XM6_HFP, Some("WH-1000XM6 Hands-Free AG"), None, Some(true), Some(50))]);
        assert_eq!(j[0].name, "WH-1000XM6");
        assert_eq!(join(&[node(XM6_HFP, None, None, Some(true), Some(50))])[0].name, "Bluetooth device");
    }

    #[test]
    fn connected_without_a_level_is_not_listed() {
        assert!(devices(join(&[node(XM6_DEVICE, Some("X"), None, Some(true), None)])).is_empty());
        assert!(devices(join(&[node(XM6_DEVICE, Some("X"), None, None, Some(50))])).is_empty());
    }

    #[test]
    fn property_values_are_read_by_their_type() {
        assert_eq!(parse_level(3, &[94]), Some(94));
        assert_eq!(parse_level(3, &[101]), None);
        assert_eq!(parse_level(7, &[94]), None);
        assert_eq!(parse_level(3, &[]), None);
        assert_eq!(parse_bool(17, &[0xFF]), Some(true));
        assert_eq!(parse_bool(17, &[0]), Some(false));
        assert_eq!(parse_bool(3, &[1]), None);
        assert_eq!(parse_u32(7, &0x240404u32.to_le_bytes()), Some(0x240404));
        assert_eq!(parse_u32(7, &[1, 2]), None);
        let s: Vec<u8> = "Pad\0".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        assert_eq!(parse_string(18, &s).as_deref(), Some("Pad"));
        assert_eq!(parse_string(7, &s), None);
        assert_eq!(parse_string(18, &[0, 0]), None);
    }

    #[test]
    fn duplicates_of_the_apps_devices() {
        let app = names(&["Razer Naga V2 Pro", "Logitech G915 TKL", "Mouse"]);
        assert!(is_duplicate("Razer Naga V2 Pro", &app));
        assert!(is_duplicate("  razer naga v2 pro ", &app));
        // One contains the other (4 characters or more).
        assert!(is_duplicate("Naga V2 Pro", &app));
        assert!(is_duplicate("Razer Naga V2 Pro Dongle", &app));
        // Too short to count as a part, but equal still matches.
        assert!(!is_duplicate("Pro", &names(&["Razer Naga V2 Pro"])));
        assert!(is_duplicate("Mouse", &app));
        assert!(!is_duplicate("Mou", &names(&["Mouse"])));
        assert!(!is_duplicate("WH-1000XM6", &app));
        assert!(!is_duplicate("", &app) && !is_duplicate("x", &names(&[""])));
    }

    #[test]
    fn visible_hides_duplicates_obeys_the_setting_and_sorts() {
        let found = [sys("Pad", 4), sys("WH-1000XM6", 94), sys("Naga V2 Pro", 100), sys("Keychron K3", 40), sys("alpha", 41)];
        let app = names(&["Razer Naga V2 Pro"]);
        let v = visible(&found, true, &app);
        assert_eq!(v.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["alpha", "Keychron K3", "Pad", "WH-1000XM6"]);
        assert!(visible(&found, false, &app).is_empty());
    }

    #[test]
    fn address_edge_cases() {
        // Lower case hex.
        assert_eq!(address_of("bthenum\\dev_58186221e9f9\\x").as_deref(), Some("58186221E9F9"));
        // Too short, malformed, empty.
        assert_eq!(address_of("BTHENUM\\DEV_58186221E9\\1&2"), None);
        assert_eq!(address_of(""), None);
        assert_eq!(address_of("DEV_"), None);
        // Non-ASCII text must not panic.
        assert_eq!(address_of("BTHENUM\\DEV_яяяяяяяя\\1"), None);
    }

    #[test]
    fn several_service_nodes_and_an_le_node_join_into_one_device() {
        let hfp2 = "BTHENUM\\{0000110B-0000-1000-8000-00805F9B34FB}_VID&0002054C_PID&0DF3\\8&2A3B4C5D&0&58186221E9F9_C00000000";
        let nodes = [
            node(hfp2, Some("WH-1000XM6"), None, None, None),
            node(XM6_HFP, Some("WH-1000XM6 Hands-Free"), None, None, Some(90)),
            node("BTHLE\\DEV_58186221E9F9\\1&2&3", None, None, Some(true), Some(91)),
        ];
        let j = join(&nodes);
        assert_eq!(j.len(), 1);
        assert!(j[0].connected);
        assert_eq!(j[0].percent, Some(90)); // the first node that has a level
        assert_eq!(j[0].name, "WH-1000XM6");
        // An explicit "no" with nothing else known stays disconnected.
        assert!(!join(&[node(XM6_DEVICE, Some("X"), None, Some(false), Some(5)), node(XM6_HFP, None, None, None, None)])[0].connected);
    }

    fn phantom(mut n: Node) -> Node {
        n.present = Some(false);
        n
    }

    const XB33_DEVICE: &str = "BTHENUM\\DEV_6C4760E7DAEF\\9&bfcbf6&0&BLUETOOTHDEVICE_6C4760E7DAEF";
    const XB33_HFP: &str = "BTHENUM\\{0000111E-0000-1000-8000-00805F9B34FB}_VID&00010094_PID&0004\\9&bfcbf6&0&6C4760E7DAEF_C00000000";

    #[test]
    fn a_level_on_a_not_present_service_node_counts_for_a_connected_device() {
        let hfp = phantom(node(XB33_HFP, Some("SRS-XB33 Hands-Free AG"), None, None, Some(10)));
        let list = devices(join(&[node(XB33_DEVICE, Some("SRS-XB33"), None, Some(true), None), hfp.clone()]));
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].name.as_str(), list[0].percent), ("SRS-XB33", 10));
        // Disconnected: the stale level stays hidden.
        assert!(devices(join(&[node(XB33_DEVICE, Some("SRS-XB33"), None, Some(false), None), hfp])).is_empty());
    }

    #[test]
    fn a_present_level_wins_over_a_not_present_one() {
        let old = phantom(node(XB33_HFP, None, None, None, Some(10)));
        let new = node("BTHLE\\DEV_6C4760E7DAEF\\1&2&3", None, None, Some(true), Some(70));
        assert_eq!(join(&[old.clone(), new.clone()])[0].percent, Some(70));
        assert_eq!(join(&[new, old])[0].percent, Some(70));
    }

    #[test]
    fn a_not_present_device_node_is_not_connected() {
        assert!(devices(join(&[phantom(node(XB33_DEVICE, Some("Old"), None, None, Some(30)))])).is_empty());
        // Radio off or dongle out while connected: the nodes go away still saying "connected".
        let stale = [phantom(node(XB33_DEVICE, Some("Old"), None, Some(true), None)), phantom(node(XB33_HFP, None, None, None, Some(30)))];
        assert!(devices(join(&stale)).is_empty());
    }

    #[test]
    fn plain_name_strips_the_suffix_only_at_the_end() {
        assert_eq!(plain_name("Buds HANDS-FREE AG"), "Buds");
        assert_eq!(plain_name("Buds Hands-Free"), "Buds");
        assert_eq!(plain_name("Hands-Free Buds"), "Hands-Free Buds");
    }

    #[test]
    fn duplicate_corner_cases() {
        // Exactly 4 characters counts as a part; 3 does not; case does not matter.
        assert!(is_duplicate("NAGA", &names(&["Razer Naga V2"])));
        assert!(!is_duplicate("Nag", &names(&["Razer Naga V2"])));
        assert!(is_duplicate("Razer Naga V2 Pro Dongle", &names(&["x", "naga v2"])));
        assert!(!is_duplicate("   ", &names(&["   ", "a"])));
        assert!(!is_duplicate("WH-1000XM6", &[]));
    }

    #[test]
    fn visible_with_equal_names_orders_by_id() {
        let (mut a, mut b) = (sys("Pad", 1), sys("pad", 2));
        a.id = "system-B".into();
        b.id = "system-A".into();
        let v = visible(&[a, b], true, &[]);
        assert_eq!(v.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["system-A", "system-B"]);
    }

    #[test]
    fn demo_spec_is_parsed_leniently() {
        let v = parse_demo("WH-1000XM6:headphones:72, Pad:gamepad:4,bad,Other:what:200,x:mouse:abc");
        assert_eq!(
            v.iter().map(|d| (d.name.as_str(), d.class, d.percent)).collect::<Vec<_>>(),
            [("WH-1000XM6", SysClass::Headphones, 72), ("Pad", SysClass::Gamepad, 4), ("Other", SysClass::Other, 100)]
        );
        assert!(parse_demo("").is_empty());
    }
}
