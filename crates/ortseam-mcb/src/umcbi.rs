//! ORTEC's UMCBI library, loaded at run time.
//!
//! `Mcbcio32.dll` is a 32-bit in-process DLL, which is why this lives in its own
//! executable rather than inside ortseam: a 64-bit process cannot load it. The
//! signatures below are transcribed from ORTEC's own `mcbcio32.h`, and the
//! calling convention is `WINAPI`, which on x86 is `stdcall`.
//!
//! It is loaded with `LoadLibrary` rather than linked, so that a machine with no
//! ORTEC software installed gets a sentence explaining what is missing instead
//! of an executable that refuses to start.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

/// An open detector. ORTEC's `HDET`, which is an opaque pointer.
pub type Hdet = *mut c_void;

type Bool32 = i32;

// The handful of Win32 calls needed to load a DLL by hand. Declared here rather
// than pulling in a Windows crate: three functions is not worth a dependency.
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryA(name: *const c_char) -> *mut c_void;
    fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> Bool32;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    fn GetLastError() -> u32;
    fn SetDllDirectoryA(path: *const c_char) -> Bool32;
}

/// The module, preferring the copy already in the process.
///
/// `mcbloc32.dll` is normally pulled in beside `Mcbcio32.dll` from the pinned
/// directory; asking for the loaded copy first means a later caller cannot
/// fetch a different one off the ordinary search path. The flag says whether
/// this call added a reference - the caller balances it with `FreeLibrary`,
/// and must not free a handle it did not add a reference to.
unsafe fn module_in_process(name: &CStr) -> (*mut c_void, bool) {
    unsafe {
        let module = GetModuleHandleA(name.as_ptr());
        if !module.is_null() {
            return (module, false);
        }
        (LoadLibraryA(name.as_ptr()), true)
    }
}

macro_rules! signatures {
    ($($field:ident: fn($($arg:ty),*) -> $ret:ty, $symbol:literal;)*) => {
        /// The subset of UMCBI ortseam uses, resolved from the DLL.
        #[allow(non_snake_case)]
        pub struct Umcbi {
            $(pub $field: unsafe extern "system" fn($($arg),*) -> $ret,)*
        }

        impl Umcbi {
            /// Resolves every symbol, naming the first one that is missing.
            unsafe fn resolve(module: *mut c_void) -> Result<Self, String> {
                unsafe {
                    Ok(Self {
                        $($field: {
                            let name = concat!($symbol, "\0");
                            let address = GetProcAddress(module, name.as_ptr().cast());
                            if address.is_null() {
                                return Err(format!(
                                    "{} does not export {}",
                                    LIBRARY, $symbol
                                ));
                            }
                            // Both sides are named, so the compiler checks the
                            // shape of what the address is being called as.
                            std::mem::transmute::<
                                *mut c_void,
                                unsafe extern "system" fn($($arg),*) -> $ret,
                            >(address)
                        },)*
                    })
                }
            }
        }
    };
}

signatures! {
    startup: fn() -> Bool32, "MIOStartup";
    debug: fn(c_int) -> c_int, "MIODebug";
    cleanup: fn() -> Bool32, "MIOCleanup";
    get_revision: fn() -> c_int, "MIOGetRevision";
    get_last_error: fn(*mut c_int, *mut c_int) -> c_int, "MIOGetLastError";
    get_config_max: fn(*const c_char, *mut c_int) -> Bool32, "MIOGetConfigMax";
    get_config_name: fn(c_int, *const c_char, c_int, *mut c_char, *mut u32, *mut Bool32)
        -> Bool32, "MIOGetConfigName";
    open_detector: fn(c_int, *const c_char, *const c_char) -> Hdet, "MIOOpenDetector";
    close_detector: fn(Hdet) -> Bool32, "MIOCloseDetector";
    comm: fn(Hdet, *const c_char, *const c_char, *const c_char, c_int, *mut c_char, *mut c_int)
        -> Bool32, "MIOComm";
    get_data: fn(Hdet, u16, u16, *mut u32, *mut u16, *mut u32, *mut u32, *const c_char)
        -> *mut u32, "MIOGetData";
    get_detector_length: fn(Hdet) -> u16, "MIOGetDetLength";
    get_detector_mcb: fn(Hdet) -> c_int, "MIOGetDetMCB";
    get_type: fn(Hdet, *mut c_char, c_int) -> *const c_char, "MIOGetTypeEx";
    is_active: fn(Hdet) -> Bool32, "MIOIsActive";
    get_start_time: fn(Hdet, *mut i32) -> i32, "MIOGetStartTime";
}

/// The library ortseam loads.
pub const LIBRARY: &str = "Mcbcio32.dll";

impl Umcbi {
    /// Loads UMCBI, looking where ORTEC's installer and its data directory put
    /// it before giving up.
    ///
    /// A named directory wins; then the ordinary search path, which finds a
    /// system-wide CONNECTIONS install; then the two places the library itself
    /// uses. The directory matters beyond `Mcbcio32.dll`, because that pulls in
    /// `mcbloc32.dll` from beside it, which in turn loads the transport add-ins
    /// named in `MCBLOC32.INI`.
    pub fn load(directory: Option<&str>) -> Result<Self, String> {
        let mut tried: Vec<String> = Vec::new();
        for candidate in Self::search_path(directory) {
            match Self::load_from(candidate.as_deref()) {
                Ok(library) => return Ok(library),
                Err(error) => tried.push(error),
            }
        }
        // A 64-bit build can never load it, however many directories are tried,
        // and Windows reports that as the same "module not found" it reports for
        // a missing file. Saying so beats sending someone to look for a DLL that
        // is sitting right there.
        if cfg!(target_pointer_width = "64") {
            return Err(format!(
                "this is a 64-bit build of the bridge and {LIBRARY} is 32-bit, so it can \
                 never load. Build it with --target i686-pc-windows-msvc."
            ));
        }
        Err(format!(
            "could not load {LIBRARY}. ORTEC's CONNECTIONS libraries must be installed, \
             or their directory named with --umcbi-dir. Tried: {}",
            tried.join("; ")
        ))
    }

    /// Where to look for the library, in order.
    fn search_path(directory: Option<&str>) -> Vec<Option<String>> {
        let mut places: Vec<Option<String>> = vec![directory.map(str::to_string), None];
        // Where the local layer keeps its data, and where a hand-installed copy
        // therefore belongs; and where ORTEC's own installer puts the program
        // files. Both are asked for by name rather than hardcoded to a drive.
        for (variable, tail) in [
            ("ProgramData", r"ORTEC shared\UMCBI"),
            ("CommonProgramFiles(x86)", r"ORTEC shared\UMCBI"),
            ("CommonProgramFiles", r"ORTEC shared\UMCBI"),
        ] {
            if let Ok(root) = std::env::var(variable) {
                places.push(Some(format!(r"{root}\{tail}")));
            }
        }
        places
    }

    /// One attempt, from one directory.
    fn load_from(directory: Option<&str>) -> Result<Self, String> {
        unsafe {
            let where_from = directory.unwrap_or("the search path").to_string();
            match directory {
                Some(directory) => {
                    if !std::path::Path::new(directory).is_dir() {
                        return Err(format!("{directory} is not a directory"));
                    }
                    let wide = CString::new(directory).map_err(|_| "the path has a NUL in it")?;
                    SetDllDirectoryA(wide.as_ptr());
                }
                None => {
                    SetDllDirectoryA(std::ptr::null());
                }
            }
            let name = CString::new(LIBRARY).expect("a literal without NUL");
            let module = LoadLibraryA(name.as_ptr());
            if module.is_null() {
                // A failed attempt leaves nothing pinned for the next one.
                SetDllDirectoryA(std::ptr::null());
                return Err(format!("{where_from} (Windows error {})", GetLastError()));
            }
            let resolved = Self::resolve(module);
            if resolved.is_err() {
                // A library missing a symbol is not the library; unload it
                // rather than leaking a reference per attempt, and unpin the
                // directory it came from.
                FreeLibrary(module);
                SetDllDirectoryA(std::ptr::null());
            }
            // On success the directory stays pinned deliberately:
            // `mcbloc32.dll` and the transport add-ins it names are loaded
            // lazily from beside `Mcbcio32.dll` at first use, and restoring
            // the search path now would send those loads elsewhere.
            resolved
        }
    }

    /// Closes the library, as UMCBI asks callers to before exiting.
    pub fn close(&self) {
        // SAFETY: resolved from the DLL, takes no arguments.
        unsafe { (self.cleanup)() };
    }

    /// The second at which the acquisition started, as a Unix time.
    ///
    /// `lpCurrentTime` is what the caller believes the time is now: the
    /// instrument keeps its clock as an interval, and the library turns that
    /// into a date against the host's. Passing zero yields a start time near
    /// the epoch rather than an error, which is what made this worth a comment.
    pub fn start_time(&self, detector: Hdet, now: i64) -> Option<i64> {
        let mut time = now as i32;
        // SAFETY: the pointer is to a live local. The return value only says
        // whether the call worked; the answer comes back through the pointer.
        let ok = unsafe { (self.get_start_time)(detector, &mut time) };
        // An instrument with nothing to report leaves the value alone, so what
        // comes back is the time that went in. That is not a start time, and
        // stamping it on a spectrum would date the file to the moment it was
        // read rather than the moment it was counted. The one case this also
        // discards - a count genuinely started within the current second - is
        // indistinguishable through this API (the answer is derived from the
        // `now` that went in), and the value lost is the second the caller
        // already knows.
        if ok == 0 || i64::from(time) == now {
            return None;
        }
        Some(i64::from(time))
    }

    /// Opens the library for use, as UMCBI requires before anything else.
    pub fn open(&self) -> Result<(), String> {
        // SAFETY: resolved from the DLL, takes no arguments.
        if unsafe { (self.startup)() } == 0 {
            return Err(format!("MIOStartup failed: {}", self.error()));
        }
        Ok(())
    }

    /// The last error, as UMCBI's macro and micro codes with a name.
    pub fn error(&self) -> String {
        let (mut macro_code, mut micro_code) = (0, 0);
        // SAFETY: both pointers are to live locals.
        unsafe { (self.get_last_error)(&mut macro_code, &mut micro_code) };
        format!(
            "{} (macro {macro_code}, micro {micro_code})",
            describe(macro_code)
        )
    }

    /// How many detectors the master configuration list holds.
    pub fn detector_count(&self) -> Result<i32, String> {
        let mut count = 0;
        // The empty list name means the master list. A null pointer here takes
        // the whole process down inside the library rather than returning
        // false, so it is never passed.
        let ok = unsafe { (self.get_config_max)(c"".as_ptr(), &mut count) };
        if ok == 0 {
            return Err(format!("MIOGetConfigMax failed: {}", self.error()));
        }
        Ok(count)
    }

    /// A configured detector's name, as MCB Configuration recorded it.
    pub fn detector_name(&self, number: i32) -> Result<String, String> {
        let mut name = vec![0_i8; 256];
        let mut id = 0_u32;
        let mut outdated = 0;
        // SAFETY: the buffer is 256 bytes and its length is passed alongside.
        let ok = unsafe {
            (self.get_config_name)(
                number,
                c"".as_ptr(),
                name.len() as c_int,
                name.as_mut_ptr(),
                &mut id,
                &mut outdated,
            )
        };
        if ok == 0 {
            return Err(format!(
                "MIOGetConfigName({number}) failed: {}",
                self.error()
            ));
        }
        Ok(from_c(&name))
    }

    /// Opens a detector by its configured number.
    pub fn open_detector(&self, number: i32) -> Result<Hdet, String> {
        // SAFETY: the empty list name means the master list, and an unlocked
        // instrument needs no authorisation string.
        let handle = unsafe { (self.open_detector)(number, c"".as_ptr(), c"".as_ptr()) };
        if handle.is_null() {
            return Err(format!(
                "MIOOpenDetector({number}) failed: {}",
                self.error()
            ));
        }
        Ok(handle)
    }

    /// Sends one command and returns the instrument's reply.
    ///
    /// The two empty strings are `lpszAuth` and `lpszPass` - authorisation and
    /// the detector password, which an unlocked instrument does not ask for.
    pub fn command(&self, detector: Hdet, command: &str) -> Result<String, String> {
        let text = CString::new(command).map_err(|_| "the command has a NUL in it")?;
        let mut reply = vec![0_i8; 1024];
        let mut length = 0;
        // SAFETY: the reply buffer's length is passed as nMaxResp.
        let ok = unsafe {
            (self.comm)(
                detector,
                text.as_ptr(),
                c"".as_ptr(),
                c"".as_ptr(),
                reply.len() as c_int,
                reply.as_mut_ptr(),
                &mut length,
            )
        };
        if ok == 0 {
            return Err(format!("{command:?} failed: {}", self.error()));
        }
        Ok(from_c(&reply))
    }

    /// Reads channels, with the masks the driver reports for them.
    ///
    /// Each channel word carries its counts and, on instruments that support
    /// regions, a flag saying the channel is inside one. Which bits are which
    /// is the driver's to say, so the masks are applied rather than assumed.
    pub fn read(&self, detector: Hdet, start: u16, count: u16) -> Result<Vec<u64>, String> {
        let mut buffer = vec![0_u32; count as usize];
        let (mut returned, mut data_mask, mut roi_mask) = (0_u16, 0_u32, 0_u32);
        // SAFETY: the buffer holds `count` words, which is what is asked for.
        let result = unsafe {
            (self.get_data)(
                detector,
                start,
                count,
                buffer.as_mut_ptr(),
                &mut returned,
                &mut data_mask,
                &mut roi_mask,
                c"".as_ptr(),
            )
        };
        if result.is_null() {
            return Err(format!("MIOGetData failed: {}", self.error()));
        }
        // A mask of zero means the instrument did not report one; take the word.
        let mask = if data_mask == 0 { u32::MAX } else { data_mask };
        buffer.truncate(returned as usize);
        Ok(buffer.iter().map(|word| (word & mask) as u64).collect())
    }

    /// The instrument's memory size in channels.
    pub fn length(&self, detector: Hdet) -> u16 {
        // SAFETY: takes an open handle and returns a plain word.
        unsafe { (self.get_detector_length)(detector) }
    }

    /// The MCB number the local layer addresses this detector by.
    ///
    /// Not the same as the detector number: the detector list is a pick list,
    /// and this is the instrument behind an entry in it. It is what names the
    /// section of `MCBLOC32.INI` holding the calibration.
    pub fn mcb_number(&self, detector: Hdet) -> i32 {
        // SAFETY: takes an open handle and returns a plain int.
        unsafe { (self.get_detector_mcb)(detector) }
    }

    /// The instrument's model string, as it reports it.
    pub fn model(&self, detector: Hdet) -> String {
        let mut text = vec![0_i8; 128];
        // SAFETY: the buffer's length is passed alongside it.
        unsafe { (self.get_type)(detector, text.as_mut_ptr(), text.len() as c_int) };
        from_c(&text)
    }

    /// Whether a count is running.
    pub fn is_counting(&self, detector: Hdet) -> bool {
        // SAFETY: takes an open handle.
        unsafe { (self.is_active)(detector) != 0 }
    }
}

/// Where ORTEC's local layer keeps its configuration on this machine.
///
/// `mcbloc32.dll` decides these paths itself, from the system's Common Files
/// folder, and exports them. Asking it is better than guessing, because putting
/// `MCBCON32.INI` anywhere else leaves the detector list empty with no
/// explanation.
pub fn local_paths() -> Vec<(&'static str, String)> {
    let mut found = Vec::new();
    // SAFETY: mcbloc32 normally sits in the process already, beside Mcbcio32;
    // `module_in_process` takes that copy first, so a call before `Umcbi::load`
    // cannot pull a different one off the search path. Each of these exports
    // takes no arguments and returns a static NUL-terminated string, copied
    // out before any unload.
    unsafe {
        let name = CString::new("mcbloc32.dll").expect("a literal without NUL");
        let (module, added) = module_in_process(&name);
        if module.is_null() {
            return found;
        }
        for (label, symbol) in [
            ("MCBLOC32.INI", "LocalMCBLOC32INIPath"),
            ("program directory", "LocalUMCBIProgramPath"),
            ("data directory", "LocalUMCBIDataPath"),
        ] {
            let symbol = CString::new(symbol).expect("a literal without NUL");
            let address = GetProcAddress(module, symbol.as_ptr());
            if address.is_null() {
                continue;
            }
            let call: unsafe extern "system" fn() -> *const c_char = std::mem::transmute(address);
            let text = call();
            if !text.is_null() {
                found.push((label, CStr::from_ptr(text).to_string_lossy().to_string()));
            }
        }
        if added {
            FreeLibrary(module);
        }
    }
    found
}

/// What instruments the local layer can actually see, independent of any
/// configuration file.
///
/// `MCBCON32.INI` is a pick list somebody built once; it can name instruments
/// that are not here, and a copy taken from another machine names the wrong
/// ones. These come from `mcbloc32.dll` asking the transports what answered,
/// which is what ORTEC's own MCB Configuration does to build the list.
///
/// The signatures are not documented - there is no header for the local layer -
/// so they are read off the decorated names in its PDB, where the trailing
/// number is the argument size in bytes. `LocalGetMCBType@12` is three words,
/// taken here as a number and a buffer with its length.
pub fn discover() -> Vec<(i32, String)> {
    // SAFETY: mcbloc32 normally sits in the process already, beside Mcbcio32,
    // and `module_in_process` takes that copy first. Both calls below are
    // given buffers whose lengths are passed alongside them.
    unsafe {
        let name = CString::new("mcbloc32.dll").expect("a literal without NUL");
        let (module, added) = module_in_process(&name);
        if module.is_null() {
            return Vec::new();
        }
        let found = discover_in(module);
        if added {
            FreeLibrary(module);
        }
        found
    }
}

/// The walk itself, separated so every early exit still balances the load.
unsafe fn discover_in(module: *mut c_void) -> Vec<(i32, String)> {
    let mut found = Vec::new();
    unsafe {
        let highest = CString::new("LocalGetHighestMCBNumber").expect("literal");
        let kind = CString::new("LocalGetMCBType").expect("literal");
        let highest = GetProcAddress(module, highest.as_ptr());
        let kind = GetProcAddress(module, kind.as_ptr());
        if highest.is_null() || kind.is_null() {
            return found;
        }
        let highest: unsafe extern "system" fn() -> c_int = std::mem::transmute(highest);
        let kind: unsafe extern "system" fn(c_int, *mut c_char, c_int) -> Bool32 =
            std::mem::transmute(kind);
        let last = highest();
        // A silly answer means the guessed signature is wrong; stop rather than
        // walk a range that came from nowhere.
        if !(1..=9999).contains(&last) {
            return found;
        }
        for number in 1..=last {
            let mut text = vec![0_i8; 64];
            if kind(number, text.as_mut_ptr(), text.len() as c_int) != 0 {
                let model = from_c(&text);
                if !model.is_empty() {
                    found.push((number, model));
                }
            }
        }
    }
    found
}

/// The name of a UMCBI macro error code, from `mcbcio32.h`.
fn describe(code: c_int) -> &'static str {
    match code {
        0 => "no error",
        1 => "invalid argument",
        2 => "the instrument rejected it",
        3 => "an I/O error",
        4 => "out of memory",
        5 => "not authorised - the detector is locked",
        6 => "the call would block",
        7 => "interrupted",
        8 => "no context - was MIOStartup called?",
        9 => "the detector is not open",
        10 => "unexpected",
        11 => "not supported by this instrument",
        -1 => "the connection is closed",
        -2 => "timed out",
        -3 => "a communication failure",
        -4 => "too many connections",
        -5 => "a machine error",
        _ => "an unrecognised error",
    }
}

/// A NUL-terminated buffer as a String, keeping only what precedes the NUL.
fn from_c(buffer: &[i8]) -> String {
    let bytes: Vec<u8> = buffer.iter().map(|value| *value as u8).collect();
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    CStr::from_bytes_with_nul(&[&bytes[..end], &[0]].concat())
        .map(|text| text.to_string_lossy().trim().to_string())
        .unwrap_or_default()
}
