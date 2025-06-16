use parking_lot::Mutex;
use std::ffi::{ c_char, CStr };

use rrplug::{ prelude::* };
use rrplug::{
    bindings::squirrelclasstypes::ScriptContext,
    call_sq_function,
    high::squirrel::compile_string,
    mid::{squirrel::SQVM_UI},
    bindings::squirreldatatypes::HSquirrelVM,
};

use std::{ ops::DerefMut };

use crate::PLUGIN;

use crate::engine::{ ExecuteConCommand, GetAddress, ENGINE_FUNCTIONS };

pub static JOIN_HANDLER_FUNCTION: Mutex<JoinHandler> = Mutex::new(default_join_handler);

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
    pub fn set_secret(&self, secret: *const c_char, match_id: *const c_char, ip: *const c_char) -> IniviteHandlerResult {
        if secret.is_null() {
            return IniviteHandlerResult::NullSecret;
        }

        if match_id.is_null() {
            return IniviteHandlerResult::NullSecret;
        }

        if ip.is_null() {
            return IniviteHandlerResult::NullSecret;
        }

        let Some(secret) = (unsafe { CStr::from_ptr(secret) }).to_str().ok() else {
            return IniviteHandlerResult::NonUtf8Secret;
        };

        let Some(match_id) = (unsafe { CStr::from_ptr(match_id) }).to_str().ok() else {
            return IniviteHandlerResult::NonUtf8Secret;
        };

        let Some(ip) = (unsafe { CStr::from_ptr(ip) }).to_str().ok() else {
            return IniviteHandlerResult::NonUtf8Secret;
        };

        let activity = &mut PLUGIN.wait().activity.lock();
        activity.secrets.join = Some(secret.to_string());
        activity.match_id = Some(match_id.to_string());
        activity.server_address = Some(ip.to_string());

        IniviteHandlerResult::Ok
    }

    /// removes the secret which destroys the party invite
    pub fn clear_secret(&self) {
        let activity = &mut PLUGIN.wait().activity.lock();

        activity.match_id = None;
        activity.server_address = None;
        activity.secrets.r#match = None;
        activity.secrets.join = None;
        activity.secrets.spectate = None;
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

// need to eventually handle loopback here for listen servers, maybe even a port scan to check if it's port-forwarded properly
#[rrplug::sqfunction(VM = "UI", ExportName = "SetJoinSecret")]
pub fn set_secret(is_lobby: bool) -> Result<(), String> {
    let plugin = crate::PLUGIN.wait();
    let mut invite_lock = plugin.invite_handler.lock();
    let invite_handler = invite_lock.deref_mut();

    let ip = match GetAddress() {
        Some(addr) => addr,
        None => return Err("Failed to get address".to_string()),
    };

    match ip.as_str() {
        // need to use net_local_adr here in the future and get the socket port properly
        "loopback" =>
            return Ok(()),//Err("Cannot set join secret for loopback address".to_string()),
        "unknown" =>
            return Err("Cannot set join secret for unknown address".to_string()),
        _ => {}
    }

    let cvar_serverfilter = ConVarStruct::find_convar_by_name("serverFilter", engine_token)
        .map_err(|_| "Failed to find serverfilter convar".to_string())?;
    let cvar_match_partysub = ConVarStruct::find_convar_by_name("match_partySub", engine_token)
        .map_err(|_| "Failed to find match_partySub convar".to_string())?;
    let is_northstar = cvar_serverfilter.get_value_bool();

    let secret = if is_northstar {
        let cvar_ns_last_tried_server_id = ConVarStruct::find_convar_by_name("ns_last_tried_server_id", engine_token)
            .map_err(|_| "Failed to find ns_last_tried_server_id convar".to_string())?;
        let server_id = cvar_ns_last_tried_server_id.get_value_string();
        if server_id.is_empty() {
            format!("l:{}", "loopback")
        } else {
            format!("n:{}", server_id)
        } 
    } else {
        if cvar_match_partysub.get_value_string().is_empty() && ip != "loopback" {
            return Ok(());
        }

        format!("v:{}", cvar_match_partysub.get_value_string())
    };

    let match_id = {
        let party_sub = cvar_match_partysub.get_value_string();
        let trimmed = party_sub.rsplitn(2, '_').nth(1).unwrap_or(&party_sub);
        trimmed.to_string()
    };

    //log::info!("Setting join secret: {}, match: {}, ip: {}", secret, match_id, ip);

    invite_handler.set_secret(
        std::ffi::CString
            ::new(secret)
            .map_err(|_| "Failed to convert secret to CString".to_string())?
            .as_ptr(),
        std::ffi::CString
            ::new(match_id)
            .map_err(|_| "Failed to convert match_id to CString".to_string())?
            .as_ptr(),
        std::ffi::CString
            ::new(ip)
            .map_err(|_| "Failed to convert ip to CString".to_string())?
            .as_ptr(),
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

    let is_northstar = secret.starts_with("n");
    let is_vanilla = secret.starts_with("v");

    if !is_northstar && !is_vanilla {
        log::error!("Invalid join secret format: {}", secret);
        return;
    }

    let value = if is_northstar {
        secret.trim_start_matches("n:")
    } else {
        secret.trim_start_matches("v:")
    };

    if secret.contains(';') {
        log::error!("Invalid join secret value: {}", value);
        return;
    }

    if is_vanilla {
        ExecuteConCommand(&format!("ns_join_room {}\n", value)).unwrap_or_else(|err| {
            log::error!("Failed to execute join command: {}\n", err);
        });
    } else if is_northstar {
        log::error!("Joining Northstar servers is not supported yet: {}", value);
    } else {
        log::error!("Unknown join secret type: {}", value);
    }
}
