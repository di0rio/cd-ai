//! AppContainer sandbox. `std::process::Command` cannot start a process inside a container, so
//! the command goes through a launcher: this same executable, started with [`LAUNCH_FLAG`], which
//! creates the contained process with the caller's stdio, waits for it and exits with its code.
//! The caller keeps an ordinary `Child` (pipes, timeout, tree kill) and never touches Win32.
//!
//! The container sees only what its SID was granted: the workspace (modify), the package caches
//! (modify) and the toolchains on the user's PATH (read/execute). Grants are ACL entries for the
//! container SID alone, written once and checked before being written again.
//!
//! One grant needs an administrator: listing `C:\` and `C:\Users`. git, Rust and Node resolve the
//! real name of every path by walking its parents (`GetFinalPathNameByHandle`), and those two
//! folders let no AppContainer list them. [`setup`] (the `cd-ai sandbox-setup` command, run once
//! as administrator) adds that listing; until then the sandbox reports itself unavailable.

use super::*;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::process::Stdio;
use std::ptr::{null, null_mut};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, HLOCAL, INVALID_HANDLE_VALUE, LocalFree,
    SetHandleInformation,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSidToSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
    GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SetEntriesInAclW,
    SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName, GetAppContainerFolderPath,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, EqualSid, FreeSid, GetAce, GetAclInformation,
    GetSecurityDescriptorControl, InitializeSecurityDescriptor, NO_INHERITANCE, OBJECT_INHERIT_ACE,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_AUTO_INHERITED, SE_DACL_PROTECTED, SECURITY_CAPABILITIES,
    SECURITY_DESCRIPTOR, SUB_CONTAINERS_AND_OBJECTS_INHERIT, SetFileSecurityW,
    SetSecurityDescriptorControl, SetSecurityDescriptorDacl,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_TRAVERSE, SYNCHRONIZE,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

/// First argument that turns this executable into the launcher.
pub const LAUNCH_FLAG: &str = "--cd-ai-sandbox-launch";
const CONTAINER_NAME: &str = "cd-ai.sandbox";
/// Exit code of a launcher that could not start the command at all.
const LAUNCH_FAILED: i32 = 126;
/// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`.
const PROFILE_EXISTS: i32 = 0x800700B7_u32 as i32;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

const READ_EXECUTE: u32 = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
const MODIFY: u32 = READ_EXECUTE | FILE_GENERIC_WRITE | DELETE | FILE_DELETE_CHILD;
/// What resolving a path's real name needs from each parent folder.
const LIST: u32 = FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE;

pub const SETUP_HINT: &str = "rode uma vez, num terminal de administrador: cd-ai sandbox-setup";

static LAUNCHER: OnceLock<PathBuf> = OnceLock::new();

pub fn init() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(OsStr::new(LAUNCH_FLAG)) {
        let argv: Vec<OsString> = args.collect();
        let code = match run_contained(&argv) {
            Ok(code) => code as i32,
            Err(message) => {
                eprintln!("cd-ai sandbox: {message}");
                LAUNCH_FAILED
            }
        };
        std::process::exit(code);
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = LAUNCHER.set(exe);
    }
}

pub fn probe() -> SandboxStatus {
    let result = container_starts().and_then(|container| {
        // The profile's parents: every workspace under it needs them listable.
        match super::home_dir() {
            Some(home) => ancestors_listable(container, &home),
            None => Ok(()),
        }
    });
    let works = result.is_ok();
    SandboxStatus {
        available: works,
        filesystem: works,
        network_block: works,
        platform: "windows".to_string(),
        detail: match result {
            Ok(()) => "AppContainer (Windows)".to_string(),
            Err(reason) => format!("sandbox do Windows indisponível: {reason}"),
        },
    }
}

/// The launcher is registered and a real process starts inside the container.
fn container_starts() -> Result<&'static Container, String> {
    let launcher = LAUNCHER
        .get()
        .ok_or_else(|| "o executável não registrou o launcher do sandbox".to_string())?;
    let container = container()?;
    // `cmd.exe` is readable by every AppContainer.
    let status = Command::new(launcher)
        .args([LAUNCH_FLAG, "cmd", "/c", "rem"])
        .current_dir(&container.folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(container)
    } else {
        Err(format!("o teste do AppContainer falhou ({status})"))
    }
}

pub fn command(program: &str, exec: &SandboxExec) -> io::Result<Command> {
    // An approved `network` command runs as the user: inside the container it would still be cut
    // off from loopback, the credential manager and `~/.ssh`, so it could not do its job.
    if !super::status().available || exec.allow_network {
        return Ok(Command::new(program));
    }
    let command = contained_command(program, exec)?;
    // A workspace outside the profile (another drive) may sit under a folder only an
    // administrator can open to the container.
    let container = container().map_err(io::Error::other)?;
    ancestors_listable(container, &exec.workspace).map_err(io::Error::other)?;
    Ok(command)
}

/// The launcher command, whatever the probe said. `command` is the one to use; this is the seam
/// the end to end test uses on a machine where `sandbox-setup` never ran.
pub fn contained_command(program: &str, exec: &SandboxExec) -> io::Result<Command> {
    let (Some(launcher), Ok(container)) = (LAUNCHER.get(), container()) else {
        return Err(io::Error::other("o sandbox do Windows não está pronto"));
    };
    prepare_host(container);
    grant_workspace(container, &exec.workspace)?;
    let mut command = Command::new(launcher);
    command.arg(LAUNCH_FLAG).arg(program);
    Ok(command)
}

/// The one step that needs an administrator: lets the container list the folders above the
/// user's profile (and above `extra`), which no user can change. Only the folder itself, never
/// what is inside it. Returns one line per folder.
pub fn setup(extra: &[PathBuf]) -> Result<Vec<String>, String> {
    let container = container()?;
    let mut targets: Vec<PathBuf> = Vec::new();
    for path in super::home_dir().iter().chain(extra) {
        for ancestor in path.ancestors().skip(1) {
            if !targets.iter().any(|seen| seen == ancestor) {
                targets.push(ancestor.to_path_buf());
            }
        }
    }
    let mut report = Vec::new();
    let mut failed = false;
    for target in &targets {
        match grant_here(target, &container.sid, LIST) {
            Ok(()) => report.push(format!("ok: {}", target.display())),
            Err(reason) => {
                failed = true;
                report.push(format!("falhou: {reason}"));
            }
        }
    }
    if failed {
        report.push("rode o comando num terminal aberto como administrador".to_string());
        return Err(report.join("\n"));
    }
    Ok(report)
}

struct Container {
    /// `S-1-15-2-...`, derived from [`CONTAINER_NAME`].
    sid: String,
    /// `%LOCALAPPDATA%\Packages\cd-ai.sandbox\AC`: the one folder the container owns.
    folder: PathBuf,
}

fn container() -> Result<&'static Container, String> {
    static CONTAINER: OnceLock<Result<Container, String>> = OnceLock::new();
    CONTAINER
        .get_or_init(open_container)
        .as_ref()
        .map_err(Clone::clone)
}

/// Creates the profile on first use and derives it afterwards.
fn open_container() -> Result<Container, String> {
    let name = wide(OsStr::new(CONTAINER_NAME));
    let display = wide(OsStr::new("cd-ai sandbox"));
    let mut sid: PSID = null_mut();
    unsafe {
        let created = CreateAppContainerProfile(
            name.as_ptr(),
            display.as_ptr(),
            display.as_ptr(),
            null(),
            0,
            &mut sid,
        );
        if created < 0 {
            if created != PROFILE_EXISTS {
                return Err(format!("CreateAppContainerProfile falhou ({created:#x})"));
            }
            let derived = DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid);
            if derived < 0 {
                return Err(format!("o perfil do AppContainer não abriu ({derived:#x})"));
            }
        }
        let text = sid_to_string(sid);
        FreeSid(sid);
        let sid = text?;
        let mut folder = null_mut();
        let found = GetAppContainerFolderPath(wide(OsStr::new(&sid)).as_ptr(), &mut folder);
        if found < 0 {
            return Err(format!(
                "a pasta do AppContainer não foi encontrada ({found:#x})"
            ));
        }
        let path = PathBuf::from(from_wide_ptr(folder));
        CoTaskMemFree(folder as *const c_void);
        Ok(Container { sid, folder: path })
    }
}

/// Read access to the toolchains the user runs from their profile, write access to the package
/// caches. Once per process; a grant that fails only makes that tool fail inside the container.
fn prepare_host(container: &Container) {
    static PREPARED: OnceLock<()> = OnceLock::new();
    PREPARED.get_or_init(|| {
        let mut readable: Vec<PathBuf> = Vec::new();
        if let Some(home) = super::home_dir() {
            if let Some(path) = std::env::var_os("PATH") {
                readable.extend(
                    std::env::split_paths(&path)
                        .filter(|dir| dir.starts_with(&home) && dir.is_dir()),
                );
            }
            readable.push(home.join(".gitconfig"));
            readable.push(home.join(".config").join("git"));
            readable.push(
                std::env::var_os("RUSTUP_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".rustup")),
            );
        }
        readable.extend(super::cargo_home());
        for path in readable.iter().filter(|path| path.exists()) {
            list_ancestors(path, &container.sid);
            let _ = grant_tree(path, &container.sid, READ_EXECUTE);
        }
        for cache in super::cache_roots() {
            list_ancestors(&cache, &container.sid);
            let _ = grant_tree(&cache, &container.sid, MODIFY);
        }
    });
}

fn grant_workspace(container: &Container, workspace: &Path) -> io::Result<()> {
    static GRANTED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
    let mut granted = GRANTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let granted = granted.get_or_insert_with(HashSet::new);
    if granted.contains(workspace) {
        return Ok(());
    }
    list_ancestors(workspace, &container.sid);
    grant_tree(workspace, &container.sid, MODIFY).map_err(|reason| {
        io::Error::other(format!(
            "não foi possível liberar o workspace para o sandbox: {reason}"
        ))
    })?;
    granted.insert(workspace.to_path_buf());
    Ok(())
}

/// Lets the container list the folders above `path`, the ones the user may change. The names in
/// those folders become visible to the container; their contents do not.
fn list_ancestors(path: &Path, sid: &str) {
    for ancestor in path.ancestors().skip(1) {
        let _ = grant_here(ancestor, sid, LIST);
    }
}

/// Every folder above `path` lists for the container; the error names the first that does not.
fn ancestors_listable(container: &Container, path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors().skip(1) {
        let listable = with_sid(&container.sid, |sid| {
            read_dacl(ancestor, |dacl| unsafe {
                has_entry(dacl, sid, LIST, false)
            })
        })??;
        if !listable {
            return Err(format!(
                "o container não consegue listar {}; {SETUP_HINT}",
                ancestor.display()
            ));
        }
    }
    Ok(())
}

/// Allow entry inherited by everything below a directory. Skipped when it is already there:
/// rewriting a directory's ACL this way walks the whole tree beneath it.
fn grant_tree(path: &Path, sid: &str, access: u32) -> Result<(), String> {
    let is_dir = path.is_dir();
    let name = wide(path.as_os_str());
    with_sid(sid, |sid| {
        read_dacl(path, |dacl| unsafe {
            if has_entry(dacl, sid, access, is_dir) {
                return Ok(());
            }
            let updated = merged_dacl(dacl, sid, access, is_dir)?;
            let written = SetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                updated,
                null(),
            );
            LocalFree(updated as HLOCAL);
            if written == 0 {
                Ok(())
            } else {
                Err(format!(
                    "{}: ACL não gravada (erro {written})",
                    path.display()
                ))
            }
        })
    })??
}

/// Allow entry on this one folder, not inherited. Written with `SetFileSecurityW`, which, unlike
/// `SetNamedSecurityInfoW`, does not walk the tree below: on `C:\Users` that walk takes minutes.
fn grant_here(path: &Path, sid: &str, access: u32) -> Result<(), String> {
    let name = wide(path.as_os_str());
    with_sid(sid, |sid| {
        read_descriptor(path, |dacl, descriptor| unsafe {
            if has_entry(dacl, sid, access, false) {
                return Ok(());
            }
            let mut control = 0u16;
            let mut revision = 0u32;
            GetSecurityDescriptorControl(descriptor, &mut control, &mut revision);
            let updated = merged_dacl(dacl, sid, access, false)?;
            let mut fresh: SECURITY_DESCRIPTOR = std::mem::zeroed();
            let fresh_ptr = &mut fresh as *mut SECURITY_DESCRIPTOR as PSECURITY_DESCRIPTOR;
            let kept = SE_DACL_PROTECTED | SE_DACL_AUTO_INHERITED;
            let ready = InitializeSecurityDescriptor(fresh_ptr, 1) != 0
                && SetSecurityDescriptorDacl(fresh_ptr, 1, updated, 0) != 0
                && SetSecurityDescriptorControl(fresh_ptr, kept, control & kept) != 0;
            let written =
                ready && SetFileSecurityW(name.as_ptr(), DACL_SECURITY_INFORMATION, fresh_ptr) != 0;
            let error = last_error(&format!("{}: ACL não gravada", path.display()));
            LocalFree(updated as HLOCAL);
            if written { Ok(()) } else { Err(error) }
        })
    })??
}

/// `dacl` plus an allow entry for `sid`; free the result with `LocalFree`.
unsafe fn merged_dacl(
    dacl: *const ACL,
    sid: PSID,
    access: u32,
    inherit: bool,
) -> Result<*mut ACL, String> {
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: access,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: if inherit {
            SUB_CONTAINERS_AND_OBJECTS_INHERIT
        } else {
            NO_INHERITANCE
        },
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid as *mut u16,
        },
    };
    let mut updated: *mut ACL = null_mut();
    let merged = unsafe { SetEntriesInAclW(1, &entry, dacl, &mut updated) };
    if merged == 0 {
        Ok(updated)
    } else {
        Err(format!("ACL não montada (erro {merged})"))
    }
}

fn read_dacl<R>(path: &Path, inspect: impl FnOnce(*const ACL) -> R) -> Result<R, String> {
    read_descriptor(path, |dacl, _| inspect(dacl))
}

fn read_descriptor<R>(
    path: &Path,
    inspect: impl FnOnce(*const ACL, PSECURITY_DESCRIPTOR) -> R,
) -> Result<R, String> {
    let name = wide(path.as_os_str());
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    let read = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if read != 0 {
        return Err(format!("{}: ACL ilegível (erro {read})", path.display()));
    }
    let result = inspect(dacl, descriptor);
    unsafe { LocalFree(descriptor as HLOCAL) };
    Ok(result)
}

/// Whether `dacl` already allows `access` to `sid` (inherited to children too, for a directory).
unsafe fn has_entry(dacl: *const ACL, sid: PSID, access: u32, needs_inheritance: bool) -> bool {
    if dacl.is_null() {
        return false;
    }
    let mut info = ACL_SIZE_INFORMATION::default();
    let read = unsafe {
        GetAclInformation(
            dacl,
            &mut info as *mut ACL_SIZE_INFORMATION as *mut c_void,
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    if read == 0 {
        return false;
    }
    let inherit_both = (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8;
    (0..info.AceCount).any(|index| unsafe {
        let mut ace: *mut c_void = null_mut();
        if GetAce(dacl, index, &mut ace) == 0 {
            return false;
        }
        let header = &*(ace as *const ACE_HEADER);
        if header.AceType != ACCESS_ALLOWED_ACE_TYPE {
            return false;
        }
        let allowed = &*(ace as *const ACCESS_ALLOWED_ACE);
        let ace_sid = &allowed.SidStart as *const u32 as PSID;
        EqualSid(ace_sid, sid) != 0
            && allowed.Mask & access == access
            && (!needs_inheritance || header.AceFlags & inherit_both == inherit_both)
    })
}

/// Launcher side: starts `argv` inside the container and waits for it.
fn run_contained(argv: &[OsString]) -> Result<u32, String> {
    let program = argv.first().ok_or("comando vazio")?;
    let application = resolve_program(program)
        .ok_or_else(|| format!("programa não encontrado: {}", program.to_string_lossy()))?;
    let container = open_container()?;
    // The user's %TEMP% is out of reach from inside; the container's own folder is not.
    let temp = container.folder.join("Temp");
    let _ = std::fs::create_dir_all(&temp);
    let mut environment = environment_block(&temp);
    let mut command_line = command_line(argv);
    with_sid(&container.sid, |sid| unsafe {
        spawn_and_wait(sid, &application, &mut command_line, &mut environment)
    })?
}

unsafe fn spawn_and_wait(
    sid: PSID,
    application: &Path,
    command_line: &mut [u16],
    environment: &mut [u16],
) -> Result<u32, String> {
    // No capabilities at all: no network, no library, no device.
    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: sid,
        Capabilities: null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let stdio = unsafe {
        [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].map(|which| GetStdHandle(which))
    };
    // Only the three stdio handles cross into the container, nothing else the launcher holds.
    let mut inherited: Vec<HANDLE> = Vec::new();
    for handle in stdio {
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE && !inherited.contains(&handle) {
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
            inherited.push(handle);
        }
    }

    let mut size = 0usize;
    unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut size) };
    // usize-backed so the list is pointer-aligned.
    let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
    let list = storage.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
    if unsafe { InitializeProcThreadAttributeList(list, 2, 0, &mut size) } == 0 {
        return Err(last_error("InitializeProcThreadAttributeList"));
    }
    let set_capabilities = unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            &capabilities as *const SECURITY_CAPABILITIES as *const c_void,
            size_of::<SECURITY_CAPABILITIES>(),
            null_mut(),
            null(),
        )
    };
    let set_handles = inherited.is_empty()
        || unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                inherited.as_ptr() as *const c_void,
                inherited.len() * size_of::<HANDLE>(),
                null_mut(),
                null(),
            )
        } != 0;
    if set_capabilities == 0 || !set_handles {
        unsafe { DeleteProcThreadAttributeList(list) };
        return Err(last_error("UpdateProcThreadAttribute"));
    }

    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdio[0];
    startup.StartupInfo.hStdOutput = stdio[1];
    startup.StartupInfo.hStdError = stdio[2];
    startup.lpAttributeList = list;

    // Killing the launcher (timeout, cancel) closes the job, which kills everything inside.
    let job = unsafe { CreateJobObjectW(null(), null()) };
    if !job.is_null() {
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
    }

    let application_name = wide(application.as_os_str());
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            application_name.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT
                | CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | CREATE_NO_WINDOW,
            environment.as_mut_ptr() as *const c_void,
            null(),
            &startup.StartupInfo,
            &mut process,
        )
    };
    let create_error = last_error(&format!(
        "não foi possível iniciar {}",
        application.display()
    ));
    unsafe { DeleteProcThreadAttributeList(list) };
    if created == 0 {
        if !job.is_null() {
            unsafe { CloseHandle(job) };
        }
        return Err(create_error);
    }
    if !job.is_null() && unsafe { AssignProcessToJobObject(job, process.hProcess) } == 0 {
        // Outside the job a timeout could leave it running: refuse instead.
        unsafe {
            TerminateProcess(process.hProcess, LAUNCH_FAILED as u32);
            CloseHandle(process.hThread);
            CloseHandle(process.hProcess);
            CloseHandle(job);
        }
        return Err(last_error("AssignProcessToJobObject"));
    }
    let mut code = LAUNCH_FAILED as u32;
    unsafe {
        ResumeThread(process.hThread);
        CloseHandle(process.hThread);
        WaitForSingleObject(process.hProcess, INFINITE);
        GetExitCodeProcess(process.hProcess, &mut code);
        CloseHandle(process.hProcess);
        if !job.is_null() {
            CloseHandle(job);
        }
    }
    Ok(code)
}

/// Finds the `.exe` the way a shell would, from PATH (absolute entries only: a relative one would
/// resolve inside the workspace). Never a `.bat`/`.cmd`: `cmd.exe` re-parses its arguments in a
/// way no quoting survives.
fn resolve_program(program: &OsStr) -> Option<PathBuf> {
    let path = Path::new(program);
    if program.to_string_lossy().contains(['/', '\\']) || path.is_absolute() {
        return executable(std::env::current_dir().ok()?.join(path));
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| executable(dir.join(program)))
}

fn executable(path: PathBuf) -> Option<PathBuf> {
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return path.is_file().then_some(path);
    }
    let mut with_exe = path.into_os_string();
    with_exe.push(".exe");
    let with_exe = PathBuf::from(with_exe);
    with_exe.is_file().then_some(with_exe)
}

/// The launcher's environment with TEMP/TMP moved into the container's folder.
fn environment_block(temp: &Path) -> Vec<u16> {
    let mut block = Vec::new();
    for (key, value) in std::env::vars_os() {
        let replaced = key.eq_ignore_ascii_case("TEMP") || key.eq_ignore_ascii_case("TMP");
        let value = if replaced { temp.as_os_str() } else { &value };
        block.extend(key.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    for key in ["TEMP", "TMP"] {
        if std::env::var_os(key).is_none() {
            block.extend(OsStr::new(key).encode_wide());
            block.push(u16::from(b'='));
            block.extend(temp.as_os_str().encode_wide());
            block.push(0);
        }
    }
    block.push(0);
    block
}

/// Joins `argv` with the quoting `CommandLineToArgvW` and the C runtime undo.
fn command_line(argv: &[OsString]) -> Vec<u16> {
    let mut line = Vec::new();
    for (index, argument) in argv.iter().enumerate() {
        if index > 0 {
            line.push(u16::from(b' '));
        }
        quote_into(argument, &mut line);
    }
    line.push(0);
    line
}

fn quote_into(argument: &OsStr, line: &mut Vec<u16>) {
    let units: Vec<u16> = argument.encode_wide().collect();
    let plain = !units.is_empty()
        && !units
            .iter()
            .any(|&unit| matches!(unit, 0x20 | 0x09 | 0x0a | 0x0b | 0x22));
    if plain {
        line.extend(units);
        return;
    }
    let backslash = u16::from(b'\\');
    let quote = u16::from(b'"');
    line.push(quote);
    let mut pending = 0usize;
    for unit in units {
        if unit == backslash {
            pending += 1;
            continue;
        }
        // Backslashes double only in front of a quote, which itself gets escaped.
        let doubled = if unit == quote {
            pending * 2 + 1
        } else {
            pending
        };
        line.extend(std::iter::repeat_n(backslash, doubled));
        line.push(unit);
        pending = 0;
    }
    line.extend(std::iter::repeat_n(backslash, pending * 2));
    line.push(quote);
}

fn with_sid<R>(sid: &str, use_sid: impl FnOnce(PSID) -> R) -> Result<R, String> {
    let text = wide(OsStr::new(sid));
    let mut psid: PSID = null_mut();
    if unsafe { ConvertStringSidToSidW(text.as_ptr(), &mut psid) } == 0 {
        return Err(last_error("SID inválido"));
    }
    let result = use_sid(psid);
    unsafe { LocalFree(psid as HLOCAL) };
    Ok(result)
}

unsafe fn sid_to_string(sid: PSID) -> Result<String, String> {
    let mut text = null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(last_error("ConvertSidToStringSidW"));
    }
    let value = unsafe { from_wide_ptr(text) }
        .to_string_lossy()
        .into_owned();
    unsafe { LocalFree(text as HLOCAL) };
    Ok(value)
}

unsafe fn from_wide_ptr(text: *const u16) -> OsString {
    let mut len = 0;
    while unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    OsString::from_wide(unsafe { std::slice::from_raw_parts(text, len) })
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

fn last_error(what: &str) -> String {
    format!("{what}: {}", io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(argv: &[&str]) -> String {
        let argv: Vec<OsString> = argv.iter().map(OsString::from).collect();
        let mut units = command_line(&argv);
        units.pop();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn command_line_quotes_like_the_c_runtime_parses() {
        assert_eq!(line(&["git", "status"]), "git status");
        assert_eq!(
            line(&["cmd", "/c", "echo x> a.txt"]),
            r#"cmd /c "echo x> a.txt""#
        );
        assert_eq!(line(&["a", ""]), r#"a """#);
        assert_eq!(line(&["a", r#"say "oi""#]), r#"a "say \"oi\"""#);
        assert_eq!(
            line(&["a", r"C:\dir with space\"]),
            r#"a "C:\dir with space\\""#
        );
        assert_eq!(line(&["a", r"C:\plain\"]), r"a C:\plain\");
        assert_eq!(line(&["a", r#"x\"y z"#]), r#"a "x\\\"y z""#);
    }

    /// `(has our entry, inherited entries)` of `path`.
    fn acl_shape(path: &Path, sid: &str) -> (bool, usize) {
        with_sid(sid, |own| {
            read_dacl(path, |dacl| unsafe {
                let mut info = ACL_SIZE_INFORMATION::default();
                GetAclInformation(
                    dacl,
                    &mut info as *mut ACL_SIZE_INFORMATION as *mut c_void,
                    size_of::<ACL_SIZE_INFORMATION>() as u32,
                    AclSizeInformation,
                );
                let inherited = (0..info.AceCount)
                    .filter(|&index| {
                        let mut ace: *mut c_void = null_mut();
                        GetAce(dacl, index, &mut ace);
                        (*(ace as *const ACE_HEADER)).AceFlags & 0x10 != 0 // INHERITED_ACE
                    })
                    .count();
                (has_entry(dacl, own, LIST, false), inherited)
            })
        })
        .unwrap()
        .unwrap()
    }

    #[test]
    fn listing_a_folder_touches_only_that_folder() {
        let Ok(container) = container() else {
            eprintln!("skip: no AppContainer profile here");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("pai");
        std::fs::create_dir_all(root.join("filho")).unwrap();
        std::fs::write(root.join("filho").join("f.txt"), "x").unwrap();
        let (_, inherited_before) = acl_shape(&root, &container.sid);

        grant_here(&root, &container.sid, LIST).unwrap();

        let (listed, inherited_after) = acl_shape(&root, &container.sid);
        assert!(listed);
        // Still inheriting from its parent: the rewrite kept the auto-inherit state.
        assert_eq!(inherited_before, inherited_after);
        // Nothing below it changed.
        assert!(!acl_shape(&root.join("filho"), &container.sid).0);
        assert!(!acl_shape(&root.join("filho").join("f.txt"), &container.sid).0);
        // And a second grant is a no-op.
        grant_here(&root, &container.sid, LIST).unwrap();
    }

    #[test]
    fn only_exe_programs_are_resolved() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tool.exe"), b"").unwrap();
        std::fs::write(dir.path().join("script.cmd"), b"").unwrap();
        assert!(executable(dir.path().join("tool")).is_some());
        assert!(executable(dir.path().join("tool.exe")).is_some());
        assert!(executable(dir.path().join("script.cmd")).is_none());
        assert!(executable(dir.path().join("script")).is_none());
    }

    #[test]
    fn environment_moves_temp_into_the_container() {
        let block = environment_block(Path::new(r"C:\container\Temp"));
        let text = String::from_utf16(&block).unwrap();
        assert!(text.ends_with("\0\0"));
        for entry in text.split('\0').filter(|entry| !entry.is_empty()) {
            if entry.to_ascii_uppercase().starts_with("TEMP=")
                || entry.to_ascii_uppercase().starts_with("TMP=")
            {
                assert!(entry.ends_with(r"C:\container\Temp"), "{entry}");
            }
        }
    }
}
