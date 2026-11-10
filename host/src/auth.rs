use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow};

const TOKEN_FILE: &str = "host-token";
const TOKEN_BYTES: usize = 32;

/// Exact match only: a web page chooses its own host name, so a prefix or suffix test could
/// be satisfied by a page on another site. `null` is what a sandboxed page sends.
pub fn origin_allowed(origin: &str) -> bool {
    matches!(origin, "tauri://localhost" | "http://tauri.localhost")
        || (cfg!(debug_assertions) && origin == "http://localhost:1420")
}

/// Constant time over the bytes, so response timing does not reveal how much of a guess was right.
pub fn tokens_match(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// Returns the token in `dir`, creating the folder and the token if they are missing.
///
/// The folder and the token are readable by the current user only: mode 0700 and 0600 on
/// Unix, and on Windows an access list that names only the current user and ignores anything
/// the parent folder would pass down.
pub fn load_or_create_token(dir: &Path) -> Result<String> {
    create_private_dir(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let path = dir.join(TOKEN_FILE);
    let token = match read_existing_token(&path) {
        Ok(Some(token)) => Ok(token),
        Ok(None) => create_token(&path),
        Err(error) => Err(error.into()),
    };
    token.with_context(|| format!("cannot set up the token in {}", path.display()))
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    #[cfg(windows)]
    windows_acl::restrict_to_user(dir, windows_acl::Target::Folder)?;
    Ok(())
}

/// Returns `None` if there is no usable token. A crash while writing can leave a short file,
/// which is removed here rather than trusted as a guessable token.
fn read_existing_token(path: &Path) -> io::Result<Option<String>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    tighten_permissions(path)?;
    match String::from_utf8(bytes) {
        Ok(token) if is_whole_token(&token) => Ok(Some(token)),
        _ => {
            fs::remove_file(path)?;
            Ok(None)
        }
    }
}

fn is_whole_token(token: &str) -> bool {
    token.len() == TOKEN_BYTES * 2 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(unix)]
fn tighten_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(windows)]
fn tighten_permissions(path: &Path) -> io::Result<()> {
    windows_acl::restrict_to_user(path, windows_acl::Target::File)
}

#[cfg(not(any(unix, windows)))]
fn tighten_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn create_token(path: &Path) -> Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| anyhow!("no random bytes: {error}"))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();

    // Written beside it and renamed, so a client reading the file never sees a partial token.
    // Only the host holding the port gets here, so nothing else is writing this temp file.
    let temp = path.with_extension("tmp");
    match fs::remove_file(&temp) {
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    // Locked down while still empty, so the token is never on disk with the folder's access
    // list. The rename keeps it, as a move on the same drive keeps a file's access list.
    #[cfg(windows)]
    windows_acl::restrict_to_user(&temp, windows_acl::Target::File)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    Ok(token)
}

#[cfg(windows)]
mod windows_acl {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
    };
    use windows_sys::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetTokenInformation,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows_sys::core::PWSTR;

    #[derive(Clone, Copy)]
    pub enum Target {
        File,
        /// Also passed down to whatever is created inside it later.
        Folder,
    }

    /// Replaces the access list on `path` with one entry: full control for the current user.
    /// "Protected" stops the parent folder's entries from being inherited, so Administrators,
    /// SYSTEM and other users lose the access they would otherwise get from above.
    pub fn restrict_to_user(path: &Path, target: Target) -> io::Result<()> {
        let sid = current_user_sid()?;
        let inherit = match target {
            Target::File => "",
            Target::Folder => "OICI",
        };
        let sddl = wide(OsStr::new(&format!("D:P(A;{inherit};FA;;;{sid})")));
        let path = wide(path.as_os_str());

        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `sddl` is a NUL-terminated UTF-16 string that outlives the call, and the
        // descriptor it returns is freed below with LocalFree, as the API asks.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = apply_dacl(&path, descriptor);
        // SAFETY: `descriptor` came from the conversion above and is not used after this.
        unsafe { LocalFree(descriptor) };
        result
    }

    fn apply_dacl(path: &[u16], descriptor: PSECURITY_DESCRIPTOR) -> io::Result<()> {
        let (mut present, mut defaulted) = (0, 0);
        let mut dacl: *mut ACL = ptr::null_mut();
        // SAFETY: `descriptor` is valid; `dacl` points into it and is used only while it lives.
        let ok = unsafe {
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if present == 0 || dacl.is_null() {
            return Err(io::Error::other("the access list did not convert"));
        }
        // SAFETY: `path` is NUL-terminated UTF-16; null owner, group and SACL leave those alone.
        let status = unsafe {
            SetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(())
    }

    /// The current user's SID in text form, such as `S-1-5-21-...-1001`.
    pub fn current_user_sid() -> io::Result<String> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo handle that needs no closing; `token`
        // is closed below.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = sid_from_token(token);
        // SAFETY: `token` was opened above and is not used after this.
        unsafe { CloseHandle(token) };
        result
    }

    fn sid_from_token(token: HANDLE) -> io::Result<String> {
        let mut needed = 0u32;
        // SAFETY: a null buffer of length 0 only asks for the size, so this call fails on purpose.
        unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut needed) };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        // u64 words keep the buffer aligned for TOKEN_USER, which holds a pointer.
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: the buffer holds at least `needed` bytes.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: on success the buffer starts with a TOKEN_USER whose SID points into it.
        let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        let mut text: PWSTR = ptr::null_mut();
        // SAFETY: `sid` is valid while `buffer` lives; `text` is freed with LocalFree.
        if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = from_wide(text);
        // SAFETY: `text` came from ConvertSidToStringSidW and is not used after this.
        unsafe { LocalFree(text.cast()) };
        Ok(result)
    }

    pub fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    /// Reads a NUL-terminated UTF-16 string that Windows allocated.
    pub fn from_wide(text: PWSTR) -> String {
        let mut len = 0;
        // SAFETY: Windows returns NUL-terminated strings, so the scan stops inside the string.
        unsafe {
            while *text.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A fresh folder under the system temp folder, removed when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let mut bytes = [0u8; 8];
            getrandom::fill(&mut bytes).unwrap();
            let suffix: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            Self(std::env::temp_dir().join(format!("term-host-{name}-{suffix}")))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn token_is_created_then_reused() {
        let dir = TempDir::new("reuse");
        let first = load_or_create_token(&dir.0).unwrap();
        assert!(is_whole_token(&first));
        assert_eq!(load_or_create_token(&dir.0).unwrap(), first);
    }

    #[cfg(unix)]
    #[test]
    fn token_and_folder_are_owner_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("unix-perms");
        load_or_create_token(&dir.0).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.0), 0o700);
        assert_eq!(mode(&dir.0.join(TOKEN_FILE)), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn loose_token_is_tightened_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("unix-loose");
        load_or_create_token(&dir.0).unwrap();
        let path = dir.0.join(TOKEN_FILE);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        load_or_create_token(&dir.0).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    /// The access list as SDDL text, such as `D:PAI(A;;FA;;;S-1-5-21-...)`.
    #[cfg(windows)]
    fn dacl_text(path: &Path) -> String {
        use std::ptr;
        use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
            SDDL_REVISION_1, SE_FILE_OBJECT,
        };
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

        let path = windows_acl::wide(path.as_os_str());
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        let status = unsafe {
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(status, ERROR_SUCCESS);
        let mut text = ptr::null_mut();
        let ok = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                ptr::null_mut(),
            )
        };
        assert_ne!(ok, 0);
        let result = windows_acl::from_wide(text);
        unsafe {
            LocalFree(text.cast());
            LocalFree(descriptor);
        }
        result
    }

    /// Protected (`P` in the flags, so nothing comes down from the parent) with exactly one
    /// entry: allow, full control, the current user.
    #[cfg(windows)]
    fn assert_user_only(path: &Path, inherit: &str) {
        let text = dacl_text(path);
        let sid = windows_acl::current_user_sid().unwrap();
        let (flags, aces) = text
            .strip_prefix("D:")
            .and_then(|rest| rest.split_once('('))
            .unwrap_or_else(|| panic!("unexpected access list {text}"));
        assert!(flags.contains('P'), "not protected: {text}");
        assert_eq!(
            format!("({aces}"),
            format!("(A;{inherit};FA;;;{sid})"),
            "in {text}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn token_and_folder_are_user_only_on_windows() {
        let dir = TempDir::new("win-acl");
        load_or_create_token(&dir.0).unwrap();
        assert_user_only(&dir.0, "OICI");
        assert_user_only(&dir.0.join(TOKEN_FILE), "");
    }

    #[cfg(windows)]
    #[test]
    fn loose_token_is_tightened_on_windows() {
        // A token left by an older host, written with whatever the temp folder passes down.
        let dir = TempDir::new("win-loose");
        fs::create_dir_all(&dir.0).unwrap();
        let path = dir.0.join(TOKEN_FILE);
        fs::write(&path, "ab".repeat(TOKEN_BYTES)).unwrap();
        assert!(
            !dacl_text(&path).starts_with("D:P"),
            "temp file already protected"
        );
        load_or_create_token(&dir.0).unwrap();
        assert_user_only(&path, "");
    }
}
