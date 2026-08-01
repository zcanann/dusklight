use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub trait Transport {
    fn send_line(&mut self, line: &str) -> std::io::Result<()>;
    fn receive_line(&mut self) -> std::io::Result<Option<String>>;
}

pub struct LineTransport<R, W> {
    reader: R,
    writer: W,
}

impl<R, W> LineTransport<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }

    pub fn into_parts(self) -> (R, W) {
        (self.reader, self.writer)
    }
}

impl<R: BufRead, W: Write> Transport for LineTransport<R, W> {
    fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()
    }

    fn receive_line(&mut self) -> std::io::Result<Option<String>> {
        let mut line = String::new();
        match self.reader.read_line(&mut line)? {
            0 => Ok(None),
            _ => {
                while matches!(line.as_bytes().last(), Some(b'\n' | b'\r')) {
                    line.pop();
                }
                Ok(Some(line))
            }
        }
    }
}

/// A persistent child connected through NDJSON on stdin/stdout. Stderr is
/// inherited so diagnostics can never corrupt the protocol stream.
pub struct ProcessTransport {
    child: Child,
    lines: LineTransport<BufReader<ChildStdout>, BufWriter<ChildStdin>>,
    suspended: bool,
}

impl ProcessTransport {
    pub fn spawn(program: impl AsRef<Path>, args: &[String]) -> std::io::Result<Self> {
        Self::spawn_in(program, args, None::<&Path>)
    }

    /// Spawns a persistent protocol child in an explicit working directory.
    /// Engine workers need this because their executable, disc, and artifact
    /// paths are authenticated independently from process cwd.
    pub fn spawn_in(
        program: impl AsRef<Path>,
        args: &[String],
        working_directory: Option<impl AsRef<Path>>,
    ) -> std::io::Result<Self> {
        let mut command = Command::new(program.as_ref());
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(working_directory) = working_directory {
            command.current_dir(working_directory);
        }
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().expect("piped child stdin");
        let stdout = child.stdout.take().expect("piped child stdout");
        Ok(Self {
            child,
            lines: LineTransport::new(BufReader::new(stdout), BufWriter::new(stdin)),
            suspended: false,
        })
    }

    pub fn child_id(&self) -> u32 {
        self.child.id()
    }

    /// Returns kernel plus user CPU consumed by this exact owned child.
    pub fn process_cpu_micros(&self) -> std::io::Result<Option<u64>> {
        process_control::process_cpu_micros(&self.child)
    }

    /// Stops every thread in an authenticated persistent child while its
    /// protocol is at an idle command boundary.
    ///
    /// The native engine owns background threads that can continue consuming
    /// CPU even while its main protocol thread is blocked on stdin. Suspending
    /// the complete process preserves its in-memory checkpoint cache and
    /// process identity without allowing unused fleet members to perturb a
    /// smaller-worker throughput sample.
    pub fn suspend_process(&mut self) -> std::io::Result<()> {
        if self.suspended {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "worker process is already suspended",
            ));
        }
        process_control::suspend(&self.child)?;
        self.suspended = true;
        Ok(())
    }

    /// Resumes a child previously stopped by [`Self::suspend_process`].
    pub fn resume_process(&mut self) -> std::io::Result<()> {
        if !self.suspended {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "worker process is not suspended",
            ));
        }
        process_control::resume(&self.child)?;
        self.suspended = false;
        Ok(())
    }
}

impl Transport for ProcessTransport {
    fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        if self.suspended {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "cannot send to a suspended worker process",
            ));
        }
        self.lines.send_line(line)
    }
    fn receive_line(&mut self) -> std::io::Result<Option<String>> {
        if self.suspended {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "cannot receive from a suspended worker process",
            ));
        }
        self.lines.receive_line()
    }
}

impl Drop for ProcessTransport {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(windows)]
mod process_control {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    #[repr(C)]
    struct FileTime {
        low_date_time: u32,
        high_date_time: u32,
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSuspendProcess(process_handle: *mut c_void) -> i32;
        fn NtResumeProcess(process_handle: *mut c_void) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetProcessTimes(
            process: *mut c_void,
            creation_time: *mut FileTime,
            exit_time: *mut FileTime,
            kernel_time: *mut FileTime,
            user_time: *mut FileTime,
        ) -> i32;
    }

    pub(super) fn process_cpu_micros(child: &Child) -> std::io::Result<Option<u64>> {
        let mut creation = FileTime {
            low_date_time: 0,
            high_date_time: 0,
        };
        let mut exit = FileTime {
            low_date_time: 0,
            high_date_time: 0,
        };
        let mut kernel = FileTime {
            low_date_time: 0,
            high_date_time: 0,
        };
        let mut user = FileTime {
            low_date_time: 0,
            high_date_time: 0,
        };
        let succeeded = unsafe {
            GetProcessTimes(
                child.as_raw_handle(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        };
        if succeeded == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let kernel_100ns =
            (u64::from(kernel.high_date_time) << 32) | u64::from(kernel.low_date_time);
        let user_100ns = (u64::from(user.high_date_time) << 32) | u64::from(user.low_date_time);
        kernel_100ns
            .checked_add(user_100ns)
            .map(|total| Some(total / 10))
            .ok_or_else(|| std::io::Error::other("worker CPU time overflowed"))
    }

    pub(super) fn suspend(child: &Child) -> std::io::Result<()> {
        let status = unsafe { NtSuspendProcess(child.as_raw_handle()) };
        nt_result(status, "suspend")
    }

    pub(super) fn resume(child: &Child) -> std::io::Result<()> {
        let status = unsafe { NtResumeProcess(child.as_raw_handle()) };
        nt_result(status, "resume")
    }

    fn nt_result(status: i32, operation: &str) -> std::io::Result<()> {
        if status >= 0 {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "cannot {operation} worker process: NTSTATUS 0x{:08x}",
                status as u32
            )))
        }
    }
}

#[cfg(unix)]
mod process_control {
    use std::process::Child;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    const SIGSTOP: i32 = 19;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const SIGCONT: i32 = 18;

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    const SIGSTOP: i32 = 17;
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    const SIGCONT: i32 = 19;

    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }

    pub(super) fn suspend(child: &Child) -> std::io::Result<()> {
        signal(child, SIGSTOP)
    }

    pub(super) fn resume(child: &Child) -> std::io::Result<()> {
        signal(child, SIGCONT)
    }

    pub(super) fn process_cpu_micros(_: &Child) -> std::io::Result<Option<u64>> {
        Ok(None)
    }

    fn signal(child: &Child, signal: i32) -> std::io::Result<()> {
        let pid = i32::try_from(child.id()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "worker process id exceeds platform signal range",
            )
        })?;
        if unsafe { kill(pid, signal) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod process_control {
    use std::process::Child;

    pub(super) fn suspend(_: &Child) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "worker process suspension is unsupported on this platform",
        ))
    }

    pub(super) fn resume(_: &Child) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "worker process resumption is unsupported on this platform",
        ))
    }

    pub(super) fn process_cpu_micros(_: &Child) -> std::io::Result<Option<u64>> {
        Ok(None)
    }
}
