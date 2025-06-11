use parking_lot::Mutex;
use rrplug::mid::utils::to_cstring;
use std::ffi::{ c_char, CStr };

use rrplug::{ offset_functions, plugin, prelude::* };
use rrplug::{
    bindings::squirrelclasstypes::ScriptContext,
    call_sq_function,
    high::squirrel::compile_string,
};

use std::{ ops::DerefMut, time::{ SystemTime, UNIX_EPOCH } };

use crate::PLUGIN;

pub static JOIN_HANDLER_FUNCTION: Mutex<JoinHandler> = Mutex::new(default_join_handler);

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

offset_functions! {
    ENGINE_FUNCTIONS + EngineFunctions for WhichDll::Engine => {
        // ccommand_tokenize = unsafe extern "C" fn(&mut Option<CCommand>, *const c_char, CmdSource) -> bool, at 0x418380;
        cbuf_add_text_type = unsafe extern "C" fn(ECommandTarget, *const c_char, CmdSource) where offset(0x1203B0);
        cbuf_execute = unsafe extern "C" fn() where offset(0x1204B0);

    }
}

type JoinHandler = extern "C" fn(*const c_char);

/// C Compatible Result Enum
///
/// fails if the string is non utf-8 or the pointer is null
#[repr(C)]
#[must_use]
pub enum IniviteHandlerResult {
    Ok,
    NullSecret,
    NonUtf8Secret,
}

fn ExecuteConCommad(cmd: &str) -> Result<(), String> {
    let cmd = to_cstring(&cmd);
    log::info!("Executing command: {}", cmd.to_string_lossy());
    unsafe {
        (ENGINE_FUNCTIONS.wait().cbuf_add_text_type)(
            ECommandTarget::FirstPlayer,
            cmd.as_ptr(),
            CmdSource::Code
        );
    }
    Ok(())
}

pub struct InviteHandler;

impl InviteHandler {
    pub fn new() -> Self {
        Self
    }

    /// Will always provide a valid null terminated string to the join handler.
    ///
    /// The join handler is called when the discord rpc client joins a party. Has to handled immediately.
    ///
    /// Discord doesn't track who is in the party. Discord only sends the secrets.
    pub fn set_join_handler(&self, handler: JoinHandler) {
        *JOIN_HANDLER_FUNCTION.lock() = handler;
    }

    /// sets a secret for party which will be provided to everyone that joins the party
    pub fn set_secret(&self, secret: *const c_char) -> IniviteHandlerResult {
        if secret.is_null() {
            return IniviteHandlerResult::NullSecret;
        }

        let Some(secret) = (unsafe { CStr::from_ptr(secret) }).to_str().ok() else {
            return IniviteHandlerResult::NonUtf8Secret;
        };

        PLUGIN.wait().activity.lock().secrets.join = Some(secret.to_string());
        IniviteHandlerResult::Ok
    }

    /// removes the secret which destroys the party invite
    pub fn clear_secret(&self) {
        let secrets = &mut PLUGIN.wait().activity.lock().secrets;

        secrets.r#match = None;
        secrets.join = None;
        secrets.spectate = None;
    }
}

#[rrplug::sqfunction(VM = "UI", ExportName = "ClearJoinSecret")]
pub fn clear_secret() -> Result<(), String> {
    let plugin = crate::PLUGIN.wait();
    let mut invite_lock = plugin.invite_handler.lock();
    let invite_handler = invite_lock.deref_mut();

    invite_handler.clear_secret();

    Ok(())
}

#[rrplug::sqfunction(VM = "UI", ExportName = "SetJoinSecret")]
pub fn set_secret(secret: String) -> Result<(), String> {
    let plugin = crate::PLUGIN.wait();
    let mut invite_lock = plugin.invite_handler.lock();
    let invite_handler = invite_lock.deref_mut();

    invite_handler.set_secret(
        std::ffi::CString
            ::new(secret)
            .map_err(|_| "Failed to convert secret to CString".to_string())?
            .as_ptr()
    );
    Ok(())
}

extern "C" fn default_join_handler(_secret: *const c_char) {
    let secret = unsafe { CStr::from_ptr(_secret) };
    let secret = match secret.to_str() {
        Ok(s) => s,
        Err(_) => {
            log::error!("Failed to convert join secret to string");
            return;
        }
    };

    log::info!("Join request received with secret: {}", secret);
    ExecuteConCommad(&format!("ns_join_room {}\n", secret)).unwrap_or_else(|err|
        log::error!("Failed to execute join command: {}\n", err)
    );
    // Call the Squirrel function to handle the join request
}
