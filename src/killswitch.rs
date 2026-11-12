//! Kill switch on the Windows Filtering Platform.
//!
//! One sublayer with a "block everything" filter plus narrow permits: loopback, sing-box.exe itself,
//! the proxy server IP, DHCP, optionally the LAN, and — once it exists — the TUN interface.
//! If sing-box dies, its TUN adapter disappears and nothing can leave the machine.
//!
//! Filters are added from a non-dynamic session: they survive a crash of ZapretGUI (no leak while
//! nobody is watching) but not a reboot (BFE drops non-persistent filters), so nobody can be
//! locked out forever. Filter keys are fixed, which lets us delete exactly our filters later.

use std::ffi::c_void;
use std::net::IpAddr;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::*;

use crate::sys::{wide, wide_path};

const SUBLAYER_KEY: GUID = GUID::from_u128(0x5a7e_c0de_3b1f_4e2a_9c41_7d2e_8f60_0001);
const FILTER_BASE: u128 = 0x5a7e_c0de_3b1f_4e2a_9c41_7d2e_8f61_0000;

// Key ranges: base rules, proxy-IP permits, TUN-interface permits.
const R_BASE: (u32, u32) = (0, 64);
const R_PROXY: (u32, u32) = (64, 80);
const R_TUN: (u32, u32) = (80, 96);

const RPC_C_AUTHN_WINNT: u32 = 10;

const LAYERS_V4: [GUID; 2] = [FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V4];
const LAYERS_V6: [GUID; 2] = [FWPM_LAYER_ALE_AUTH_CONNECT_V6, FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V6];

struct Engine(HANDLE);

impl Engine {
    fn open() -> Result<Self, String> {
        let mut h: HANDLE = null_mut();
        let r = unsafe { FwpmEngineOpen0(null(), RPC_C_AUTHN_WINNT, null(), null(), &mut h) };
        if r != 0 {
            return Err(format!("не удалось открыть WFP (0x{r:08x}) — запущена ли служба BFE?"));
        }
        Ok(Engine(h))
    }

    fn txn<T>(&self, f: impl FnOnce(&Engine) -> Result<T, String>) -> Result<T, String> {
        unsafe {
            let r = FwpmTransactionBegin0(self.0, 0);
            if r != 0 {
                return Err(format!("WFP transaction: 0x{r:08x}"));
            }
            match f(self) {
                Ok(v) => {
                    let r = FwpmTransactionCommit0(self.0);
                    if r != 0 {
                        FwpmTransactionAbort0(self.0);
                        return Err(format!("WFP commit: 0x{r:08x}"));
                    }
                    Ok(v)
                }
                Err(e) => {
                    FwpmTransactionAbort0(self.0);
                    Err(e)
                }
            }
        }
    }

    fn delete_range(&self, (from, to): (u32, u32)) {
        for i in from..to {
            let k = GUID::from_u128(FILTER_BASE + i as u128);
            unsafe {
                FwpmFilterDeleteByKey0(self.0, &k);
            }
        }
    }

    fn sublayer_exists(&self) -> bool {
        unsafe {
            let mut p: *mut FWPM_SUBLAYER0 = null_mut();
            let r = FwpmSubLayerGetByKey0(self.0, &SUBLAYER_KEY, &mut p);
            if r == 0 && !p.is_null() {
                let mut v = p as *mut c_void;
                FwpmFreeMemory0(&mut v);
                return true;
            }
            false
        }
    }

    fn add_sublayer(&self) -> Result<(), String> {
        if self.sublayer_exists() {
            return Ok(());
        }
        let mut name = wide("ZapretGUI kill switch");
        let mut desc = wide("Blocks traffic outside the proxy tunnel");
        unsafe {
            let mut sl: FWPM_SUBLAYER0 = std::mem::zeroed();
            sl.subLayerKey = SUBLAYER_KEY;
            sl.displayData.name = name.as_mut_ptr();
            sl.displayData.description = desc.as_mut_ptr();
            sl.weight = 0xFFFF;
            let r = FwpmSubLayerAdd0(self.0, &sl, null_mut());
            if r != 0 {
                return Err(format!("FwpmSubLayerAdd0: 0x{r:08x}"));
            }
        }
        Ok(())
    }

    /// Adds one filter. `conds` pointers must stay valid for the duration of the call.
    fn add(
        &self,
        key_idx: &mut u32,
        limit: u32,
        layer: GUID,
        permit: bool,
        weight: u8,
        conds: &mut [FWPM_FILTER_CONDITION0],
        name: &str,
    ) -> Result<(), String> {
        if *key_idx >= limit {
            return Err("слишком много правил kill switch".into());
        }
        let mut wname = wide(name);
        unsafe {
            let mut f: FWPM_FILTER0 = std::mem::zeroed();
            f.filterKey = GUID::from_u128(FILTER_BASE + *key_idx as u128);
            f.displayData.name = wname.as_mut_ptr();
            f.layerKey = layer;
            f.subLayerKey = SUBLAYER_KEY;
            f.weight.r#type = FWP_UINT8;
            f.weight.Anonymous.uint8 = weight;
            f.numFilterConditions = conds.len() as u32;
            f.filterCondition = if conds.is_empty() { null_mut() } else { conds.as_mut_ptr() };
            f.action.r#type = if permit { FWP_ACTION_PERMIT } else { FWP_ACTION_BLOCK };
            let mut id = 0u64;
            let r = FwpmFilterAdd0(self.0, &f, null_mut(), &mut id);
            if r != 0 {
                return Err(format!("FwpmFilterAdd0 ({name}): 0x{r:08x}"));
            }
        }
        *key_idx += 1;
        Ok(())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            FwpmEngineClose0(self.0);
        }
    }
}

// ---------------------------------------------------------------- conditions

fn cond_u32(field: GUID, mt: FWP_MATCH_TYPE, v: u32) -> FWPM_FILTER_CONDITION0 {
    unsafe {
        let mut c: FWPM_FILTER_CONDITION0 = std::mem::zeroed();
        c.fieldKey = field;
        c.matchType = mt;
        c.conditionValue.r#type = FWP_UINT32;
        c.conditionValue.Anonymous.uint32 = v;
        c
    }
}

fn cond_u8(field: GUID, v: u8) -> FWPM_FILTER_CONDITION0 {
    unsafe {
        let mut c: FWPM_FILTER_CONDITION0 = std::mem::zeroed();
        c.fieldKey = field;
        c.matchType = FWP_MATCH_EQUAL;
        c.conditionValue.r#type = FWP_UINT8;
        c.conditionValue.Anonymous.uint8 = v;
        c
    }
}

fn cond_u16(field: GUID, v: u16) -> FWPM_FILTER_CONDITION0 {
    unsafe {
        let mut c: FWPM_FILTER_CONDITION0 = std::mem::zeroed();
        c.fieldKey = field;
        c.matchType = FWP_MATCH_EQUAL;
        c.conditionValue.r#type = FWP_UINT16;
        c.conditionValue.Anonymous.uint16 = v;
        c
    }
}

fn cond_u64(field: GUID, v: *mut u64) -> FWPM_FILTER_CONDITION0 {
    unsafe {
        let mut c: FWPM_FILTER_CONDITION0 = std::mem::zeroed();
        c.fieldKey = field;
        c.matchType = FWP_MATCH_EQUAL;
        c.conditionValue.r#type = FWP_UINT64;
        c.conditionValue.Anonymous.uint64 = v;
        c
    }
}

fn cond_blob(field: GUID, v: *mut FWP_BYTE_BLOB) -> FWPM_FILTER_CONDITION0 {
    unsafe {
        let mut c: FWPM_FILTER_CONDITION0 = std::mem::zeroed();
        c.fieldKey = field;
        c.matchType = FWP_MATCH_EQUAL;
        c.conditionValue.r#type = FWP_BYTE_BLOB_TYPE;
        c.conditionValue.Anonymous.byteBlob = v;
        c
    }
}

fn cond_v4(v: *mut FWP_V4_ADDR_AND_MASK) -> FWPM_FILTER_CONDITION0 {
    unsafe {
        let mut c: FWPM_FILTER_CONDITION0 = std::mem::zeroed();
        c.fieldKey = FWPM_CONDITION_IP_REMOTE_ADDRESS;
        c.matchType = FWP_MATCH_EQUAL;
        c.conditionValue.r#type = FWP_V4_ADDR_MASK;
        c.conditionValue.Anonymous.v4AddrMask = v;
        c
    }
}

fn cond_v6(v: *mut FWP_V6_ADDR_AND_MASK) -> FWPM_FILTER_CONDITION0 {
    unsafe {
        let mut c: FWPM_FILTER_CONDITION0 = std::mem::zeroed();
        c.fieldKey = FWPM_CONDITION_IP_REMOTE_ADDRESS;
        c.matchType = FWP_MATCH_EQUAL;
        c.conditionValue.r#type = FWP_V6_ADDR_MASK;
        c.conditionValue.Anonymous.v6AddrMask = v;
        c
    }
}

fn v4mask(a: [u8; 4], prefix: u32) -> FWP_V4_ADDR_AND_MASK {
    let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
    FWP_V4_ADDR_AND_MASK { addr: u32::from_be_bytes(a), mask }
}

fn v6mask(a: [u8; 16], prefix: u8) -> FWP_V6_ADDR_AND_MASK {
    FWP_V6_ADDR_AND_MASK { addr: a, prefixLength: prefix }
}

struct AppId(*mut FWP_BYTE_BLOB);

impl AppId {
    fn of(path: &Path) -> Result<Self, String> {
        let w = wide_path(path);
        let mut p: *mut FWP_BYTE_BLOB = null_mut();
        let r = unsafe { FwpmGetAppIdFromFileName0(w.as_ptr(), &mut p) };
        if r != 0 || p.is_null() {
            return Err(format!("FwpmGetAppIdFromFileName0: 0x{r:08x}"));
        }
        Ok(AppId(p))
    }
}

impl Drop for AppId {
    fn drop(&mut self) {
        let mut v = self.0 as *mut c_void;
        unsafe { FwpmFreeMemory0(&mut v) };
    }
}

// --------------------------------------------------------------------- API

/// Is our sublayer (and therefore probably our block filter) installed?
pub fn is_active() -> bool {
    Engine::open().map(|e| e.sublayer_exists()).unwrap_or(false)
}

/// Installs the base rules: block all, permit loopback / sing-box / DHCP / (LAN).
/// Does nothing if the kill switch is already active (avoids a gap while re-adding).
pub fn enable(singbox: &Path, allow_lan: bool) -> Result<(), String> {
    let e = Engine::open()?;
    if e.sublayer_exists() {
        return Ok(());
    }
    e.delete_range((0, R_TUN.1));
    let app = AppId::of(singbox)?;
    e.txn(|e| {
        e.add_sublayer()?;
        let mut k = R_BASE.0;
        let lim = R_BASE.1;
        let lan4 = [
            v4mask([10, 0, 0, 0], 8),
            v4mask([172, 16, 0, 0], 12),
            v4mask([192, 168, 0, 0], 16),
            v4mask([169, 254, 0, 0], 16),
            v4mask([224, 0, 0, 0], 4),
            v4mask([255, 255, 255, 255], 32),
        ];
        let lan6 = [
            v6mask([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 10),
            v6mask([0xfc, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 7),
            v6mask([0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8),
        ];

        for (v6, layers) in [(false, LAYERS_V4), (true, LAYERS_V6)] {
            for layer in layers {
                e.add(&mut k, lim, layer, false, 0, &mut [], "ZapretGUI: block all")?;

                let mut c = [cond_u32(FWPM_CONDITION_FLAGS, FWP_MATCH_FLAGS_ALL_SET, FWP_CONDITION_FLAG_IS_LOOPBACK)];
                e.add(&mut k, lim, layer, true, 15, &mut c, "ZapretGUI: loopback")?;

                let mut c = [cond_blob(FWPM_CONDITION_ALE_APP_ID, app.0)];
                e.add(&mut k, lim, layer, true, 14, &mut c, "ZapretGUI: sing-box")?;

                // DHCP (v4: server port 67, v6: server port 547)
                let mut c = [cond_u8(FWPM_CONDITION_IP_PROTOCOL, 17), cond_u16(FWPM_CONDITION_IP_REMOTE_PORT, if v6 { 547 } else { 67 })];
                e.add(&mut k, lim, layer, true, 12, &mut c, "ZapretGUI: DHCP")?;

                if allow_lan {
                    if v6 {
                        for m in lan6 {
                            let mut m = m;
                            let mut c = [cond_v6(&mut m)];
                            e.add(&mut k, lim, layer, true, 10, &mut c, "ZapretGUI: LAN v6")?;
                        }
                    } else {
                        for m in lan4 {
                            let mut m = m;
                            let mut c = [cond_v4(&mut m)];
                            e.add(&mut k, lim, layer, true, 10, &mut c, "ZapretGUI: LAN")?;
                        }
                    }
                }
            }
        }
        Ok(())
    })
}

/// Permits traffic to the proxy server itself (in addition to the sing-box app permit).
pub fn set_proxy_ip(ip: IpAddr) -> Result<(), String> {
    let e = Engine::open()?;
    e.delete_range(R_PROXY);
    e.txn(|e| {
        let mut k = R_PROXY.0;
        match ip {
            IpAddr::V4(a) => {
                for layer in LAYERS_V4 {
                    let mut m = v4mask(a.octets(), 32);
                    let mut c = [cond_v4(&mut m)];
                    e.add(&mut k, R_PROXY.1, layer, true, 13, &mut c, "ZapretGUI: proxy server")?;
                }
            }
            IpAddr::V6(a) => {
                for layer in LAYERS_V6 {
                    let mut m = v6mask(a.octets(), 128);
                    let mut c = [cond_v6(&mut m)];
                    e.add(&mut k, R_PROXY.1, layer, true, 13, &mut c, "ZapretGUI: proxy server")?;
                }
            }
        }
        Ok(())
    })
}

/// Permits everything that is routed through the TUN interface with the given LUID.
pub fn set_tun(luid: u64) -> Result<(), String> {
    let e = Engine::open()?;
    e.delete_range(R_TUN);
    e.txn(|e| {
        let mut k = R_TUN.0;
        for layer in LAYERS_V4.into_iter().chain(LAYERS_V6) {
            let mut l = luid;
            let mut c = [cond_u64(FWPM_CONDITION_IP_LOCAL_INTERFACE, &mut l)];
            e.add(&mut k, R_TUN.1, layer, true, 13, &mut c, "ZapretGUI: tunnel")?;
        }
        Ok(())
    })
}

/// Removes every filter and the sublayer. Safe to call when nothing is installed.
pub fn disable() -> Result<(), String> {
    let e = Engine::open()?;
    e.delete_range((0, R_TUN.1));
    unsafe {
        FwpmSubLayerDeleteByKey0(e.0, &SUBLAYER_KEY);
    }
    if e.sublayer_exists() {
        return Err("не удалось удалить правила kill switch".into());
    }
    Ok(())
}
