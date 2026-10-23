//! User-only permissions on Windows, through a protected access-control list.

// Unsafe code is denied workspace-wide; this module calls Win32 security functions.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_FLAGS, ACL, ACL_REVISION, ACL_SIZE_INFORMATION, AclSizeInformation,
    AddAccessAllowedAceEx, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, EqualSid, GetAce,
    GetAclInformation, GetLengthSid, GetSecurityDescriptorControl, GetTokenInformation,
    InitializeAcl, OBJECT_INHERIT_ACE, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    PSID, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PCWSTR;

/// The current user's security identifier, held in the buffer `GetTokenInformation`
/// filled, which the `PSID` points into.
struct UserSid {
    buffer: Vec<u64>,
}

impl UserSid {
    /// Reads the current process's user from its access token.
    fn current() -> io::Result<Self> {
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
                .map_err(io::Error::other)?;
            let mut needed = 0u32;
            // The first call only reports the size needed; its error is expected.
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
            // u64 elements keep the buffer aligned for TOKEN_USER.
            let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
            let filled = GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                needed,
                &mut needed,
            );
            let _ = CloseHandle(token);
            filled.map_err(io::Error::other)?;
            Ok(Self { buffer })
        }
    }

    /// The security identifier itself.
    fn psid(&self) -> PSID {
        // SAFETY: `buffer` was filled by GetTokenInformation(TokenUser), so it starts
        // with a TOKEN_USER whose SID points inside the same buffer.
        unsafe { (*(self.buffer.as_ptr() as *const TOKEN_USER)).User.Sid }
    }
}

/// `path` as a NUL-terminated wide string.
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// A Win32 status as an `io::Result`.
fn check(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status.0 as i32))
    }
}

/// Replaces `path`'s access-control list with one entry, full control for the current
/// user, inherited by children, and protected so it doesn't inherit from its parent.
pub fn restrict_to_user(path: &Path) -> io::Result<()> {
    let user = UserSid::current()?;
    unsafe {
        let sid = user.psid();
        let size = std::mem::size_of::<ACL>()
            + std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            + GetLengthSid(sid) as usize;
        let mut acl_buffer = vec![0u64; size.div_ceil(8)];
        let acl = acl_buffer.as_mut_ptr() as *mut ACL;
        InitializeAcl(acl, size as u32, ACL_REVISION).map_err(io::Error::other)?;
        AddAccessAllowedAceEx(
            acl,
            ACL_REVISION,
            ACE_FLAGS(OBJECT_INHERIT_ACE.0 | CONTAINER_INHERIT_ACE.0),
            FILE_ALL_ACCESS.0,
            sid,
        )
        .map_err(io::Error::other)?;
        let name = wide(path);
        check(SetNamedSecurityInfoW(
            PCWSTR(name.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(acl),
            None,
        ))
    }
}

/// Whether `path`'s access-control list is protected and holds exactly one entry, for
/// the current user.
pub fn is_user_only(path: &Path) -> io::Result<bool> {
    let user = UserSid::current()?;
    let name = wide(path);
    unsafe {
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        check(GetNamedSecurityInfoW(
            PCWSTR(name.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut dacl),
            None,
            &mut descriptor,
        ))?;
        let result = (|| -> io::Result<bool> {
            let mut control = 0u16;
            let mut revision = 0u32;
            GetSecurityDescriptorControl(descriptor, &mut control, &mut revision)
                .map_err(io::Error::other)?;
            if control & SE_DACL_PROTECTED.0 == 0 || dacl.is_null() {
                return Ok(false);
            }
            let mut info = ACL_SIZE_INFORMATION::default();
            GetAclInformation(
                dacl,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
            .map_err(io::Error::other)?;
            if info.AceCount != 1 {
                return Ok(false);
            }
            let mut ace: *mut c_void = std::ptr::null_mut();
            GetAce(dacl, 0, &mut ace).map_err(io::Error::other)?;
            let ace = ace as *const ACCESS_ALLOWED_ACE;
            let ace_sid = PSID(&(*ace).SidStart as *const u32 as *mut c_void);
            Ok(EqualSid(ace_sid, user.psid()).is_ok())
        })();
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        result
    }
}
