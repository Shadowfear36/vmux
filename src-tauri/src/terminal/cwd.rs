//! Query the current working directory of a process on Windows.
//! Uses NtQueryInformationProcess to read the PEB → ProcessParameters → CurrentDirectory.

use std::ffi::c_void;

/// Query the current working directory of a process by PID.
/// Returns None if the process cannot be accessed or the read fails.
pub fn get_process_cwd(pid: u32) -> Option<String> {
    unsafe { get_process_cwd_inner(pid) }
}

#[repr(C)]
struct ProcessBasicInformation {
    _reserved1: *mut c_void,
    peb_base_address: *mut c_void,
    _reserved2: [*mut c_void; 2],
    _unique_process_id: usize,
    _reserved3: *mut c_void,
}

#[repr(C)]
struct UnicodeString {
    length: u16,
    _maximum_length: u16,
    buffer: *mut u16,
}

type NtQueryInfoFn = unsafe extern "system" fn(
    handle: windows::Win32::Foundation::HANDLE,
    info_class: u32,
    info: *mut c_void,
    info_length: u32,
    return_length: *mut u32,
) -> i32;

unsafe fn get_process_cwd_inner(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::*;
    use windows::Win32::System::Threading::*;
    use windows::Win32::System::Diagnostics::Debug::*;
    use windows::Win32::System::LibraryLoader::*;
    use windows::core::s;

    let handle = OpenProcess(
        PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
        false,
        pid,
    ).ok()?;

    let _guard = scopeguard(handle);

    // Load NtQueryInformationProcess from ntdll.dll
    let ntdll = GetModuleHandleA(s!("ntdll.dll")).ok()?;
    let proc_addr = GetProcAddress(ntdll, s!("NtQueryInformationProcess"))?;
    let nt_query: NtQueryInfoFn = std::mem::transmute(proc_addr);

    // Get PEB address via ProcessBasicInformation (info class 0)
    let mut pbi = std::mem::zeroed::<ProcessBasicInformation>();
    let mut ret_len: u32 = 0;
    let status = nt_query(
        handle,
        0,
        &mut pbi as *mut _ as *mut c_void,
        std::mem::size_of::<ProcessBasicInformation>() as u32,
        &mut ret_len,
    );
    if status != 0 { return None; }

    // Read ProcessParameters pointer from PEB (offset 0x20 on 64-bit)
    let params_ptr_addr = (pbi.peb_base_address as usize + 0x20) as *const c_void;
    let mut params_ptr: usize = 0;
    let mut bytes_read = 0usize;
    ReadProcessMemory(
        handle,
        params_ptr_addr,
        &mut params_ptr as *mut _ as *mut c_void,
        std::mem::size_of::<usize>(),
        Some(&mut bytes_read),
    ).ok()?;

    // Read CurrentDirectory.DosPath (UNICODE_STRING at offset 0x38 on 64-bit)
    let cwd_addr = (params_ptr + 0x38) as *const c_void;
    let mut cwd_ustr = std::mem::zeroed::<UnicodeString>();
    ReadProcessMemory(
        handle,
        cwd_addr,
        &mut cwd_ustr as *mut _ as *mut c_void,
        std::mem::size_of::<UnicodeString>(),
        Some(&mut bytes_read),
    ).ok()?;

    if cwd_ustr.length == 0 || cwd_ustr.buffer.is_null() { return None; }

    // Read the actual path string
    let char_count = cwd_ustr.length as usize / 2;
    let mut buf = vec![0u16; char_count];
    ReadProcessMemory(
        handle,
        cwd_ustr.buffer as *const c_void,
        buf.as_mut_ptr() as *mut c_void,
        cwd_ustr.length as usize,
        Some(&mut bytes_read),
    ).ok()?;

    let path = String::from_utf16_lossy(&buf);
    let path = path.trim_end_matches('\\').to_string();
    if path.is_empty() { None } else { Some(path) }
}

/// RAII guard to close the process handle on drop.
struct HandleGuard(windows::Win32::Foundation::HANDLE);
impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe { let _ = windows::Win32::Foundation::CloseHandle(self.0); }
    }
}

fn scopeguard(h: windows::Win32::Foundation::HANDLE) -> HandleGuard {
    HandleGuard(h)
}
