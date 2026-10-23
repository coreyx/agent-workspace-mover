//! `libproc` and volume queries on macOS (facts.md, F-10).

// Unsafe code is denied workspace-wide; this module calls libproc and pathconf.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::paths::CaseRule;

// libc 0.2 lacks these two pieces of <sys/proc_info.h>, so they're declared here with
// the SDK's layout. `vnode_info_path` itself is in libc.

/// `PROC_PIDFDVNODEPATHINFO`: asks `proc_pidfdinfo` for a file descriptor's path.
const PROC_PIDFDVNODEPATHINFO: libc::c_int = 2;

/// `struct proc_fileinfo`: how a file descriptor was opened. Only its size matters here.
#[repr(C)]
#[allow(dead_code)]
struct ProcFileInfo {
    fi_openflags: u32,
    fi_status: u32,
    fi_offset: libc::off_t,
    fi_type: i32,
    fi_guardflags: u32,
}

/// `struct vnode_fdinfowithpath`: a file descriptor's open state and its vnode's path.
#[repr(C)]
struct VnodeFdInfoWithPath {
    pfi: ProcFileInfo,
    pvip: libc::vnode_info_path,
}

/// The paths of the vnodes (files and folders) `pid` has open.
pub fn open_paths(pid: u32) -> io::Result<Vec<PathBuf>> {
    let pid = pid as libc::c_int;
    let fd_size = std::mem::size_of::<libc::proc_fdinfo>();
    // Ask for the size first, then read; descriptors opened in between are missed,
    // which a process that's busy opening files can't avoid.
    let needed =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return Err(io::Error::last_os_error());
    }
    let mut fds =
        vec![unsafe { std::mem::zeroed::<libc::proc_fdinfo>() }; needed as usize / fd_size + 16];
    let bytes = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            fds.as_mut_ptr().cast(),
            (fds.len() * fd_size) as libc::c_int,
        )
    };
    if bytes <= 0 {
        return Err(io::Error::last_os_error());
    }
    let mut paths = Vec::new();
    for fd in &fds[..bytes as usize / fd_size] {
        if fd.proc_fdtype != libc::PROX_FDTYPE_VNODE as u32 {
            continue;
        }
        let mut info: VnodeFdInfoWithPath = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<VnodeFdInfoWithPath>() as libc::c_int;
        let read = unsafe {
            libc::proc_pidfdinfo(
                pid,
                fd.proc_fd,
                PROC_PIDFDVNODEPATHINFO,
                (&mut info as *mut VnodeFdInfoWithPath).cast(),
                size,
            )
        };
        if read == size {
            let path = unsafe { CStr::from_ptr(info.pvip.vip_path.as_ptr().cast()) };
            paths.push(PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes())));
        }
    }
    Ok(paths)
}

/// Whether names on `path`'s volume are case-sensitive. A volume that won't say is
/// treated as insensitive, the macOS default (design.md, F-10).
pub fn case_rule(path: &Path) -> CaseRule {
    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return CaseRule::Insensitive;
    };
    match unsafe { libc::pathconf(c_path.as_ptr(), libc::_PC_CASE_SENSITIVE) } {
        1 => CaseRule::Sensitive,
        _ => CaseRule::Insensitive,
    }
}
