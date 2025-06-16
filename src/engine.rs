use rrplug::{ offset_functions, prelude::* };
use rrplug::mid::utils::to_cstring;
use std::ffi::{ c_char, c_void };

#[derive(Debug, Clone)]
#[repr(C)]
pub enum CmdSource {
    // Added to the console buffer by gameplay code.  Generally unrestricted.
    Code,

    // Sent from code via engine->ClientCmd, which is restricted to commands visible
    // via FCVAR_GAMEDLL_FOR_REMOTE_CLIENTS.
    ClientCmd,

    // Typed in at the console or via a user key-bind.  Generally unrestricted, although
    // the client will throttle commands sent to the server this way to 16 per second.
    UserInput,

    // Came in over a net connection as a clc_stringcmd
    // host_client will be valid during this state.
    //
    // Restricted to FCVAR_GAMEDLL commands (but not convars) and special non-ConCommand
    // server commands hardcoded into gameplay code (e.g. "joingame")
    NetClient,

    // Received from the server as the client
    //
    // Restricted to commands with FCVAR_SERVER_CAN_EXECUTE
    NetServer,

    // Being played back from a demo file
    //
    // Not currently restricted by convar flag, but some commands manually ignore calls
    // from this source.  FIXME: Should be heavily restricted as demo commands can come
    // from untrusted sources.
    DemoFile,

    // Invalid value used when cleared
    Invalid = -1,
}

#[derive(Debug, Clone)]
#[repr(C)]
pub enum ECommandTarget {
    FirstPlayer = 0,
    LastPlayer = 1,
    Server = 2,

    Count,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum NetAdrType {
    Null = 0,
    Loopback = 1,
    Ip = 2,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct NetAdrT {
    pub r#type: NetAdrType,
    pub ip: [u8; 16],    // IPv6, IPv4 is mapped as described
    pub port: u16,
}

offset_functions! {
    ENGINE_FUNCTIONS + EngineFunctions for WhichDll::Engine => {
        // ccommand_tokenize = unsafe extern "C" fn(&mut Option<CCommand>, *const c_char, CmdSource) -> bool, at 0x418380;
        cbuf_add_text_type = unsafe extern "C" fn(ECommandTarget, *const c_char, CmdSource) where offset(0x1203B0);
        cbuf_execute = unsafe extern "C" fn() where offset(0x1204B0);
        get_base_local_client = unsafe extern "C" fn() -> *mut c_void where offset(0x78200);
        net_local_adr = *mut NetAdrT where offset(0x13FA38A0);
        net_getudpport = unsafe extern "C" fn(*const c_void) -> u16 where offset(0x21A4B0);
        sv_socket = *const c_void where offset(0x12A53D4C);
    }
}

pub fn ExecuteConCommand(cmd: &str) -> Result<(), String> {
    let cmd = to_cstring(&cmd);
    unsafe {
        (ENGINE_FUNCTIONS.wait().cbuf_add_text_type)(
            ECommandTarget::FirstPlayer,
            cmd.as_ptr(),
            CmdSource::Code
        );
    }
    Ok(())
}

pub fn GetAddress() -> Option<String> {
    let base_local_client = unsafe { (ENGINE_FUNCTIONS.wait().get_base_local_client)() };
    if base_local_client.is_null() {
        return None
    }

    let net_chan = unsafe {
        let net_chan_ptr = (base_local_client as *const u8).add(96) as *const usize;
        *(net_chan_ptr) as *const u8
    };
    
    if net_chan.is_null() {
        return None
    }

    let ns_addr_ptr = unsafe { net_chan.add(13964) as *const NetAdrT };
    if ns_addr_ptr.is_null() {
        return None
    }

    let ns_addr = unsafe { std::ptr::read_unaligned(ns_addr_ptr) };
    let addr_type = ns_addr.r#type;
    let ip = ns_addr.ip;
    let port = ns_addr.port;

    let ip_str = match addr_type {
        NetAdrType::Loopback => {
            // let sv_socket = unsafe { ENGINE_FUNCTIONS.wait().sv_socket };
            // let local_port = unsafe { (ENGINE_FUNCTIONS.wait().net_getudpport)(sv_socket) };
            // let local_adr = unsafe { ENGINE_FUNCTIONS.wait().net_local_adr };
            // let local_ip = unsafe { std::ptr::read_unaligned(local_adr) };
            // format!("{}.{}.{}.{}:{}", local_ip.ip[12], local_ip.ip[13], local_ip.ip[14], local_ip.ip[15], local_port)
            "loopback".to_string()
        }
        NetAdrType::Ip => {
            // byte order here is probably wrong
            format!("{}.{}.{}.{}:{}", ip[12], ip[13], ip[14], ip[15], port)
        }
        _ => "unknown".to_string(),
    };

    Some(ip_str)
}