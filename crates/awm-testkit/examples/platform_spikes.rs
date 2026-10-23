//! Spikes that answer design.md's platform facts F-8 to F-10 (tasks.md, TASK-4).
//!
//! These are throwaway measurements, not product code: the platform layer itself is
//! built in TASK-8. Each subcommand prints what it found, and the results are recorded
//! in spec/agent-workspace-mover/facts.md.
//!
//! - `rm <file count>` (Windows, F-8): times the Restart Manager over that many files,
//!   one of them held open by this process, and checks it names this process.
//! - `job` (Windows, F-9): whether this process is in a job, and the job's breakaway
//!   and kill-on-close limits.
//! - `schedule <marker> <seconds>` (Windows, F-9): registers a one-off Task Scheduler
//!   task for the current user that runs `payload`, and waits for it to start.
//! - `detach <marker> <seconds>` (Windows, F-9): starts `payload` as a detached process
//!   that tries to break away from this process's job.
//! - `unschedule` (Windows): deletes the tasks `schedule` registered.
//! - `payload <marker> <seconds>`: writes "started" to the marker file, waits, then
//!   writes "finished", so a caller can tell whether it outlived its starter.
//! - `mac` (macOS, F-10): reads another process's working folder and open files through
//!   `libproc`, and a volume's case sensitivity.

// Unsafe code is denied workspace-wide; these spikes call OS interfaces directly.
#![allow(unsafe_code)]

use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// Dispatches to the subcommand named by the first argument.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    match arg(0).as_str() {
        "payload" => payload(Path::new(&arg(1)), arg(2).parse().unwrap_or(30)),
        #[cfg(windows)]
        "rm" => windows_spikes::restart_manager(arg(1).parse().expect("a file count")),
        #[cfg(windows)]
        "job" => windows_spikes::job_info(),
        #[cfg(windows)]
        "schedule" => windows_spikes::schedule(Path::new(&arg(1)), arg(2).parse().unwrap_or(30)),
        #[cfg(windows)]
        "detach" => windows_spikes::detach(Path::new(&arg(1)), arg(2).parse().unwrap_or(30)),
        #[cfg(windows)]
        "unschedule" => windows_spikes::unschedule(),
        #[cfg(target_os = "macos")]
        "mac" => mac_spikes::run(),
        other => eprintln!("unknown or unsupported subcommand '{other}' on this OS"),
    }
}

/// Records its own start and finish in `marker`, `seconds` apart, so whoever started it
/// can check afterwards whether it survived its starter exiting.
fn payload(marker: &Path, seconds: u64) {
    let note = |text: String| {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(marker)
            .expect("the marker file opens");
        writeln!(file, "{text}").expect("the marker file is writable");
    };
    note(format!("started pid={}", std::process::id()));
    std::thread::sleep(Duration::from_secs(seconds));
    note("finished".to_owned());
}

#[cfg(windows)]
mod windows_spikes {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, VARIANT_FALSE};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::System::JobObjects::{
        IsProcessInJob, JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, QueryInformationJobObject,
    };
    use windows::Win32::System::RestartManager::{
        CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources,
        RmStartSession,
    };
    use windows::Win32::System::TaskScheduler::{
        IExecAction, ITaskService, TASK_ACTION_EXEC, TASK_CREATE_OR_UPDATE,
        TASK_LOGON_INTERACTIVE_TOKEN, TASK_TRIGGER_REGISTRATION, TaskScheduler,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;
    use windows::Win32::System::Variant::VARIANT;
    use windows::core::{BOOL, BSTR, Interface, PCWSTR, PWSTR};

    /// The Task Scheduler folder and name prefix for spike tasks, so `unschedule` can
    /// find every one of them.
    const TASK_PREFIX: &str = "awm-spike-";

    /// UTF-16 with a terminating NUL, as Win32 expects.
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// F-8: creates `count` files, holds one open, and times registering them all with
    /// the Restart Manager and asking which processes hold them.
    pub fn restart_manager(count: usize) {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let started = Instant::now();
        let paths: Vec<_> = (0..count)
            .map(|i| {
                let path = dir.path().join(format!("f{i:06}.txt"));
                std::fs::write(&path, b"x").expect("the file is written");
                path
            })
            .collect();
        println!("created {count} files in {:?}", started.elapsed());
        let _held = std::fs::File::open(&paths[count / 2]).expect("the held file opens");

        let wides: Vec<Vec<u16>> = paths.iter().map(|p| wide(p)).collect();
        for batch in [1_000usize, 10_000, count] {
            let batch = batch.min(count);
            let mut session = 0u32;
            let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
            let result = unsafe { RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())) };
            assert_eq!(result, ERROR_SUCCESS, "RmStartSession");

            let started = Instant::now();
            for chunk in wides.chunks(batch) {
                let pointers: Vec<PCWSTR> = chunk.iter().map(|w| PCWSTR(w.as_ptr())).collect();
                let result = unsafe { RmRegisterResources(session, Some(&pointers), None, None) };
                assert_eq!(result, ERROR_SUCCESS, "RmRegisterResources");
            }
            let registered = started.elapsed();

            let mut needed = 0u32;
            let mut returned = 0u32;
            let mut reasons = 0u32;
            let first =
                unsafe { RmGetList(session, &mut needed, &mut returned, None, &mut reasons) };
            let mut infos = vec![RM_PROCESS_INFO::default(); needed as usize];
            returned = needed;
            let second = if first == ERROR_MORE_DATA {
                unsafe {
                    RmGetList(
                        session,
                        &mut needed,
                        &mut returned,
                        Some(infos.as_mut_ptr()),
                        &mut reasons,
                    )
                }
            } else {
                first
            };
            let listed = started.elapsed();
            let names_this_process = infos[..returned as usize]
                .iter()
                .any(|info| info.Process.dwProcessId == std::process::id());
            unsafe {
                let _ = RmEndSession(session);
            }
            println!(
                "batch {batch:>6}: registered in {registered:?}, listed in {listed:?} \
                 (result {:?}, {returned} holder(s), names this process: {names_this_process})",
                second.0
            );
        }
    }

    /// F-9: whether this process runs inside a job, and whether that job lets a child
    /// break away or kills everything when it closes.
    pub fn job_info() {
        let mut in_job = BOOL(0);
        unsafe { IsProcessInJob(GetCurrentProcess(), None, &mut in_job) }.expect("IsProcessInJob");
        println!("in a job: {}", in_job.as_bool());
        if !in_job.as_bool() {
            return;
        }
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        let queried = unsafe {
            QueryInformationJobObject(
                None,
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as *mut _,
                std::mem::size_of_val(&info) as u32,
                None,
            )
        };
        match queried {
            Ok(()) => {
                let flags = info.BasicLimitInformation.LimitFlags;
                println!(
                    "breakaway allowed: {}",
                    flags.contains(JOB_OBJECT_LIMIT_BREAKAWAY_OK)
                );
                println!(
                    "silent breakaway: {}",
                    flags.contains(JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK)
                );
                println!(
                    "kill on job close: {}",
                    flags.contains(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
                );
            }
            Err(error) => println!("job limits unreadable: {error}"),
        }
    }

    /// F-9: registers a task for the current user that starts `payload` at once, waits
    /// up to 20 seconds for it to start, and reports whether it did.
    pub fn schedule(marker: &Path, seconds: u64) {
        let name = format!("{TASK_PREFIX}{}", std::process::id());
        match register(&name, marker, seconds) {
            Ok(()) => println!("registered task {name} as a standard user"),
            Err(error) => {
                println!("registering the task failed: {error}");
                return;
            }
        }
        report_start(marker);
    }

    /// Connects to the Task Scheduler as the current user.
    fn connect() -> windows::core::Result<ITaskService> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            let service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
            service.Connect(
                &VARIANT::default(),
                &VARIANT::default(),
                &VARIANT::default(),
                &VARIANT::default(),
            )?;
            Ok(service)
        }
    }

    /// The Task Scheduler's root folder, where spike tasks are registered.
    fn root_folder() -> windows::core::Result<windows::Win32::System::TaskScheduler::ITaskFolder> {
        unsafe { connect()?.GetFolder(&BSTR::from("\\")) }
    }

    /// Registers task `name`, which runs `payload` as soon as it's registered.
    fn register(name: &str, marker: &Path, seconds: u64) -> windows::core::Result<()> {
        let exe = std::env::current_exe().expect("this program's path");
        let service = connect()?;
        unsafe {
            let folder = service.GetFolder(&BSTR::from("\\"))?;
            let task = service.NewTask(0)?;
            let settings = task.Settings()?;
            settings.SetDisallowStartIfOnBatteries(VARIANT_FALSE)?;
            settings.SetStopIfGoingOnBatteries(VARIANT_FALSE)?;
            settings.SetExecutionTimeLimit(&BSTR::from("PT2H"))?;
            task.Principal()?
                .SetLogonType(TASK_LOGON_INTERACTIVE_TOKEN)?;
            task.Triggers()?.Create(TASK_TRIGGER_REGISTRATION)?;
            let action: IExecAction = task.Actions()?.Create(TASK_ACTION_EXEC)?.cast()?;
            action.SetPath(&BSTR::from(exe.to_string_lossy().as_ref()))?;
            action.SetArguments(&BSTR::from(format!(
                "payload \"{}\" {seconds}",
                marker.display()
            )))?;
            folder.RegisterTaskDefinition(
                &BSTR::from(name),
                &task,
                TASK_CREATE_OR_UPDATE.0,
                &VARIANT::default(),
                &VARIANT::default(),
                TASK_LOGON_INTERACTIVE_TOKEN,
                &VARIANT::default(),
            )?;
            Ok(())
        }
    }

    /// F-9: starts `payload` detached, asking to break away from this process's job,
    /// and reports whether the request was allowed and whether it started.
    pub fn detach(marker: &Path, seconds: u64) {
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let exe = std::env::current_exe().expect("this program's path");
        let spawn = |flags: u32| {
            std::process::Command::new(&exe)
                .args(["payload", &marker.to_string_lossy(), &seconds.to_string()])
                .creation_flags(flags)
                .spawn()
        };
        match spawn(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB) {
            Ok(child) => println!("started pid {} with breakaway", child.id()),
            Err(error) => {
                println!("breakaway refused ({error}); starting without it");
                match spawn(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP) {
                    Ok(child) => println!("started pid {} without breakaway", child.id()),
                    Err(error) => {
                        println!("starting failed: {error}");
                        return;
                    }
                }
            }
        }
        report_start(marker);
    }

    /// Deletes every task `schedule` registered.
    pub fn unschedule() {
        let delete_all = || -> windows::core::Result<usize> {
            let folder = root_folder()?;
            unsafe {
                let tasks = folder.GetTasks(1)?; // 1: include hidden tasks
                let mut names = Vec::new();
                for i in 1..=tasks.Count()? {
                    let name = tasks.get_Item(&VARIANT::from(i))?.Name()?.to_string();
                    if name.starts_with(TASK_PREFIX) {
                        names.push(name);
                    }
                }
                for name in &names {
                    folder.DeleteTask(&BSTR::from(name.as_str()), 0)?;
                }
                Ok(names.len())
            }
        };
        match delete_all() {
            Ok(n) => println!("deleted {n} spike task(s)"),
            Err(error) => println!("deleting spike tasks failed: {error}"),
        }
    }

    /// Waits up to 20 seconds for the payload's "started" line in `marker`.
    fn report_start(marker: &Path) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(marker)
                && let Some(line) = text.lines().find(|l| l.starts_with("started"))
            {
                println!("payload {line}");
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        println!("payload didn't start within 20 seconds");
    }
}

#[cfg(target_os = "macos")]
mod mac_spikes {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::process::Command;

    // libc 0.2 lacks these two pieces of <sys/proc_info.h>, so they're declared here with
    // the SDK's layout. `vnode_info_path` itself is in libc.

    /// `PROC_PIDFDVNODEPATHINFO`: asks `proc_pidfdinfo` for a file descriptor's path.
    const PROC_PIDFDVNODEPATHINFO: libc::c_int = 2;

    /// `struct proc_fileinfo`: how a file descriptor was opened.
    #[repr(C)]
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

    /// F-10: reads a child's working folder and open files through `libproc`, without
    /// extra rights, and reports the temporary volume's case sensitivity.
    pub fn run() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let real = std::fs::canonicalize(dir.path()).expect("the folder resolves");
        let file = real.join("held.txt");
        std::fs::write(&file, b"x").expect("the file is written");

        let mut child = Command::new("tail")
            .args(["-f", &file.to_string_lossy()])
            .current_dir(&real)
            .spawn()
            .expect("tail starts");
        std::thread::sleep(std::time::Duration::from_millis(500));
        let pid = child.id() as libc::c_int;

        let mut vnode: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
        let got = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                &mut vnode as *mut _ as *mut libc::c_void,
                size,
            )
        };
        let cwd = unsafe { CStr::from_ptr(vnode.pvi_cdir.vip_path.as_ptr().cast()) };
        println!(
            "working folder read: {} (matches: {})",
            got == size,
            cwd.to_bytes() == real.as_os_str().as_bytes()
        );

        let fd_size = std::mem::size_of::<libc::proc_fdinfo>();
        let mut fds = vec![unsafe { std::mem::zeroed::<libc::proc_fdinfo>() }; 256];
        let bytes = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDLISTFDS,
                0,
                fds.as_mut_ptr().cast(),
                (fds.len() * fd_size) as libc::c_int,
            )
        };
        let mut found = false;
        for fd in &fds[..(bytes.max(0) as usize / fd_size)] {
            if fd.proc_fdtype != libc::PROX_FDTYPE_VNODE as u32 {
                continue;
            }
            let mut info: VnodeFdInfoWithPath = unsafe { std::mem::zeroed() };
            let info_size = std::mem::size_of::<VnodeFdInfoWithPath>() as libc::c_int;
            let n = unsafe {
                libc::proc_pidfdinfo(
                    pid,
                    fd.proc_fd,
                    PROC_PIDFDVNODEPATHINFO,
                    &mut info as *mut _ as *mut libc::c_void,
                    info_size,
                )
            };
            if n == info_size {
                let path = unsafe { CStr::from_ptr(info.pvip.vip_path.as_ptr().cast()) };
                found |= path.to_bytes() == file.as_os_str().as_bytes();
            }
        }
        println!(
            "open files listed: {} fd(s); held file found: {found}",
            bytes.max(0) as usize / fd_size
        );
        let _ = child.kill();
        let _ = child.wait();

        let c_path = CString::new(real.as_os_str().as_bytes()).expect("no NUL in the path");
        let sensitive = unsafe { libc::pathconf(c_path.as_ptr(), libc::_PC_CASE_SENSITIVE) };
        println!("temporary volume case-sensitive: {sensitive} (1 yes, 0 no, -1 unknown)");
    }
}
