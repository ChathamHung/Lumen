//! Operating system integration.
//!
//! Everything here writes to `HKEY_CURRENT_USER` only, so Lumen never needs
//! administrator rights. Windows intentionally prevents an application from
//! silently taking over file associations: registering makes Lumen appear in
//! the "Open with" menu and in Windows Settings, and the user confirms the
//! final choice there (one click).

use std::path::Path;

/// Image types Lumen registers for.
pub const ASSOCIATED_EXTENSIONS: &[&str] = &[
    ".jpg", ".jpeg", ".png", ".gif", ".bmp", ".webp", ".tif", ".tiff", ".ico",
];

/// Programmatic identifier used inside the registry.
const PROG_ID: &str = "Lumen.Image";
/// Key name under `Software\RegisteredApplications`.
const APP_KEY: &str = "Lumen";

/// True when the "default photo viewer" feature is available on this platform.
pub const fn is_supported() -> bool {
    cfg!(windows)
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

/// Registers Lumen as a photo viewer for the current user.
#[cfg(windows)]
pub fn register() -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let exe = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // 1. ProgID describing how an image is opened.
    let (prog_id, _) = hkcu
        .create_subkey(format!(r"Software\Classes\{PROG_ID}"))
        .map_err(stringify)?;
    prog_id.set_value("", &"Lumen image").map_err(stringify)?;
    prog_id
        .set_value("FriendlyTypeName", &"Lumen image")
        .map_err(stringify)?;

    let (icon, _) = prog_id.create_subkey("DefaultIcon").map_err(stringify)?;
    icon.set_value("", &format!("\"{exe}\",0")).map_err(stringify)?;

    let (command, _) = prog_id
        .create_subkey(r"shell\open\command")
        .map_err(stringify)?;
    command
        .set_value("", &format!("\"{exe}\" \"%1\""))
        .map_err(stringify)?;

    // 2. Advertise the ProgID for every supported extension.
    for extension in ASSOCIATED_EXTENSIONS {
        let (key, _) = hkcu
            .create_subkey(format!(r"Software\Classes\{extension}"))
            .map_err(stringify)?;
        let (open_with, _) = key.create_subkey("OpenWithProgids").map_err(stringify)?;
        open_with.set_value(PROG_ID, &"").map_err(stringify)?;
    }

    // 3. Entry in the classic "Open with" menu.
    let (application, _) = hkcu
        .create_subkey(r"Software\Classes\Applications\lumen.exe")
        .map_err(stringify)?;
    application
        .set_value("FriendlyAppName", &"Lumen")
        .map_err(stringify)?;
    let (open_command, _) = application
        .create_subkey(r"shell\open\command")
        .map_err(stringify)?;
    open_command
        .set_value("", &format!("\"{exe}\" \"%1\""))
        .map_err(stringify)?;
    let (types, _) = application.create_subkey("SupportedTypes").map_err(stringify)?;
    for extension in ASSOCIATED_EXTENSIONS {
        types.set_value(extension, &"").map_err(stringify)?;
    }

    // 4. Registered application with capabilities - this is what the Windows
    //    "Default apps" page reads.
    let (capabilities, _) = hkcu
        .create_subkey(format!(r"Software\{APP_KEY}\Capabilities"))
        .map_err(stringify)?;
    capabilities
        .set_value("ApplicationName", &"Lumen")
        .map_err(stringify)?;
    capabilities
        .set_value(
            "ApplicationDescription",
            &"Fast, portable photo viewer built with Rust",
        )
        .map_err(stringify)?;

    let (associations, _) = capabilities
        .create_subkey("FileAssociations")
        .map_err(stringify)?;
    for extension in ASSOCIATED_EXTENSIONS {
        associations
            .set_value(extension, &PROG_ID)
            .map_err(stringify)?;
    }

    let (registered, _) = hkcu
        .create_subkey(r"Software\RegisteredApplications")
        .map_err(stringify)?;
    registered
        .set_value(APP_KEY, &format!(r"Software\{APP_KEY}\Capabilities"))
        .map_err(stringify)?;

    Ok(())
}

/// Removes everything [`register`] wrote. Safe to call when not registered.
#[cfg(windows)]
pub fn unregister() -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // Remove only our value, never the extension keys themselves.
    for extension in ASSOCIATED_EXTENSIONS {
        let path = format!(r"Software\Classes\{extension}\OpenWithProgids");
        if let Ok(key) = hkcu.open_subkey_with_flags(path, KEY_WRITE) {
            let _ = key.delete_value(PROG_ID);
        }
    }

    let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\{PROG_ID}"));
    let _ = hkcu.delete_subkey_all(r"Software\Classes\Applications\lumen.exe");
    let _ = hkcu.delete_subkey_all(format!(r"Software\{APP_KEY}"));

    if let Ok(key) = hkcu.open_subkey_with_flags(r"Software\RegisteredApplications", KEY_WRITE) {
        let _ = key.delete_value(APP_KEY);
    }
    Ok(())
}

/// True when the registration points at the current executable.
#[cfg(windows)]
pub fn is_registered() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    let Ok(registered) = hkcu.open_subkey(r"Software\RegisteredApplications") else {
        return false;
    };
    if registered.get_value::<String, _>(APP_KEY).is_err() {
        return false;
    }

    let Ok(command) = hkcu.open_subkey(format!(r"Software\Classes\{PROG_ID}\shell\open\command"))
    else {
        return false;
    };
    let Ok(command) = command.get_value::<String, _>("") else {
        return false;
    };

    // The executable may have been moved since the last registration.
    match std::env::current_exe() {
        Ok(exe) => command.contains(&exe.to_string_lossy().to_string()),
        Err(_) => true,
    }
}

/// Opens the Windows "Default apps" page so the user can pick Lumen.
#[cfg(windows)]
pub fn open_default_apps_settings() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW keeps the helper console from flashing on screen.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    std::process::Command::new("cmd")
        .args(["/C", "start", "", "ms-settings:defaultapps"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(stringify)
}

// ---------------------------------------------------------------------------
// Other platforms (kept so the project still builds everywhere)
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
pub fn register() -> Result<(), String> {
    Err("not supported on this platform".to_owned())
}

#[cfg(not(windows))]
pub fn unregister() -> Result<(), String> {
    Err("not supported on this platform".to_owned())
}

#[cfg(not(windows))]
pub fn is_registered() -> bool {
    false
}

#[cfg(not(windows))]
pub fn open_default_apps_settings() -> Result<(), String> {
    Err("not supported on this platform".to_owned())
}

// ---------------------------------------------------------------------------
// Explorer helpers (shared)
// ---------------------------------------------------------------------------

/// Opens the file manager with `path` selected.
pub fn reveal_in_explorer(path: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(not(windows))]
    {
        if let Some(parent) = path.parent() {
            open_folder(parent);
        }
    }
}

/// Opens a folder in the file manager.
pub fn open_folder(path: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

#[cfg(windows)]
fn stringify(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Minutes to add to UTC to obtain local time, read once from the registry.
///
/// Windows stores `ActiveTimeBias` as "minutes west of UTC" (already adjusted
/// for daylight saving), so the sign is flipped here. Falls back to 0 (UTC)
/// when the value cannot be read.
pub fn local_utc_offset_minutes() -> i64 {
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(|| {
        #[cfg(windows)]
        {
            use winreg::enums::HKEY_LOCAL_MACHINE;
            use winreg::RegKey;

            let key = RegKey::predef(HKEY_LOCAL_MACHINE);
            if let Ok(path) = key.open_subkey(r"SYSTEM\CurrentControlSet\Control\TimeZoneInformation")
            {
                if let Ok(bias) = path.get_value::<u32, _>("ActiveTimeBias") {
                    return -(bias as i32 as i64);
                }
            }
            0
        }
        #[cfg(not(windows))]
        {
            0
        }
    })
}
