//! Android start-up glue. On other platforms every function is a no-op.
//!
//! `rustls-platform-verifier` checks certificates with Android's own trust
//! store (so user-installed and company roots work), and it needs the Java VM
//! and app context to do that. Tauri puts both into `ndk-context` before the
//! app's setup runs; this hands them over.

/// Let TLS use Android's certificate checks. Call once at start-up, before
/// any connection is made.
///
/// # Errors
/// When the Java VM refuses the call; the message says why.
#[cfg(target_os = "android")]
pub fn init_tls() -> Result<(), String> {
    use jni::JavaVM;
    use jni::objects::JObject;

    let ctx = ndk_context::android_context();
    // SAFETY: Tauri initialises ndk-context with the process's live JavaVM
    // before app setup runs, and the VM lives as long as the process.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    vm.attach_current_thread(|env| -> Result<(), jni::errors::Error> {
        // SAFETY: ndk-context holds a global reference to the application
        // Context, valid for the life of the process. JObject has no Drop,
        // so this does not release that reference.
        let context = unsafe { JObject::from_raw(env, ctx.context().cast()) };
        rustls_platform_verifier::android::init_with_env(env, context)
    })
    .map_err(|e| e.to_string())
}

/// Nothing to do off Android.
///
/// # Errors
/// Never.
#[cfg(not(target_os = "android"))]
pub const fn init_tls() -> Result<(), String> {
    Ok(())
}
