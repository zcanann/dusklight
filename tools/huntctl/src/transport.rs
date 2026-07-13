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
}

impl ProcessTransport {
    pub fn spawn(program: impl AsRef<Path>, args: &[String]) -> std::io::Result<Self> {
        let mut child = Command::new(program.as_ref())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped child stdin");
        let stdout = child.stdout.take().expect("piped child stdout");
        Ok(Self {
            child,
            lines: LineTransport::new(BufReader::new(stdout), BufWriter::new(stdin)),
        })
    }

    pub fn child_id(&self) -> u32 {
        self.child.id()
    }
}

impl Transport for ProcessTransport {
    fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        self.lines.send_line(line)
    }
    fn receive_line(&mut self) -> std::io::Result<Option<String>> {
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
