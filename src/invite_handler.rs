use parking_lot::Mutex;
use std::ffi::{c_char, CStr};

use rrplug::prelude::*;
use rrplug::{
    bindings::squirrelclasstypes::ScriptContext, call_sq_function, high::squirrel::compile_string,
};

use std::{
    ops::DerefMut,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::PLUGIN;

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
    pub fn set_secret(&self, secret: *const c_char) -> IniviteHandlerResult {
        if secret.is_null() {
            return IniviteHandlerResult::NullSecret;
        }

        let Some(secret) = unsafe { CStr::from_ptr(secret) }.to_str().ok() else {
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
        std::ffi::CString::new(secret)
            .map_err(|_| "Failed to convert secret to CString".to_string())?
            .as_ptr(),
    );

    Ok(())
}

extern "C" fn default_join_handler(_secret: *const c_char) {}
