use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

const MAX_OUTPUT_LINES: usize = 1000;
const MAX_LINE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSegment {
    pub text: String,
    pub color: Option<[u8; 3]>,
}

pub struct TerminalSession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Arc<Mutex<VecDeque<Vec<TerminalSegment>>>>,
}

impl TerminalSession {
    pub fn spawn(root: &Path) -> Result<Self> {
        let system = native_pty_system();
        let pair = system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to create terminal PTY")?;

        let shell = std::env::var("SHELL")
            .ok()
            .filter(|shell| !shell.trim().is_empty())
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "powershell.exe".to_string()
                } else {
                    "/bin/sh".to_string()
                }
            });
        let mut command = CommandBuilder::new(shell);
        command.cwd(root);
        command.env("TERM", "dumb");
        command.env("PS1", "$ ");
        command.env("PROMPT", "$P$G");
        if cfg!(windows) {
            command.args(["-NoLogo", "-NoProfile"]);
        } else {
            command.arg("-i");
        }

        let child = pair
            .slave
            .spawn_command(command)
            .context("failed to start interactive shell")?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let output = Arc::new(Mutex::new(VecDeque::new()));
        spawn_reader(reader, Arc::clone(&output));

        Ok(Self {
            master: pair.master,
            writer,
            child,
            output,
        })
    }

    pub fn write_input(&mut self, input: &str) -> Result<()> {
        self.writer
            .write_all(input.as_bytes())
            .context("failed to write to terminal")?;
        self.writer
            .flush()
            .context("failed to flush terminal input")
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            cols: cols.max(1),
            rows: rows.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    pub fn output(&self) -> Vec<String> {
        self.output
            .lock()
            .map(|lines| {
                lines
                    .iter()
                    .map(|segments| {
                        segments
                            .iter()
                            .map(|segment| segment.text.as_str())
                            .collect::<String>()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn colored_output(&self) -> Vec<Vec<TerminalSegment>> {
        self.output
            .lock()
            .map(|lines| lines.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn is_running(&mut self) -> Result<bool> {
        Ok(self.child.try_wait()?.is_none())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    output: Arc<Mutex<VecDeque<Vec<TerminalSegment>>>>,
) {
    thread::spawn(move || {
        let mut bytes = [0; 4096];
        let mut line = Vec::new();
        let mut segments = Vec::new();
        let mut foreground = None;
        let mut bold = false;
        let mut sequence = Vec::new();
        let mut escape = 0u8;
        loop {
            let count = match reader.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            for byte in &bytes[..count] {
                match escape {
                    1 if *byte == b'[' => escape = 2,
                    1 => escape = 0,
                    2 if (0x40..=0x7e).contains(byte) => {
                        if *byte == b'm' {
                            push_segment(&mut segments, &line, terminal_color(foreground, bold));
                            line.clear();
                            apply_sgr(&sequence, &mut foreground, &mut bold);
                        } else if *byte == b'J' && sequence.as_slice() == b"2" {
                            if let Ok(mut lines) = output.lock() {
                                lines.clear();
                            }
                            line.clear();
                            segments.clear();
                        }
                        sequence.clear();
                        escape = 0;
                    }
                    2 => sequence.push(*byte),
                    _ if *byte == 0x1b => escape = 1,
                    _ if *byte == b'\n' => {
                        push_segment(&mut segments, &line, terminal_color(foreground, bold));
                        push_line(&output, std::mem::take(&mut segments));
                        line.clear();
                    }
                    _ if *byte == b'\r' => {}
                    _ if (*byte >= 0x20 || *byte == b'\t') && line.len() < MAX_LINE_BYTES => {
                        line.push(*byte)
                    }
                    _ => {}
                }
            }
        }
        if !line.is_empty() {
            push_segment(&mut segments, &line, terminal_color(foreground, bold));
            push_line(&output, segments);
        }
    });
}

fn push_segment(segments: &mut Vec<TerminalSegment>, bytes: &[u8], color: Option<[u8; 3]>) {
    if bytes.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(bytes).into_owned();
    if let Some(last) = segments.last_mut().filter(|last| last.color == color) {
        last.text.push_str(&text);
    } else {
        segments.push(TerminalSegment { text, color });
    }
}

fn push_line(output: &Mutex<VecDeque<Vec<TerminalSegment>>>, segments: Vec<TerminalSegment>) {
    if let Ok(mut lines) = output.lock() {
        lines.push_back(segments);
        while lines.len() > MAX_OUTPUT_LINES {
            lines.pop_front();
        }
    }
}

fn apply_sgr(sequence: &[u8], foreground: &mut Option<u8>, bold: &mut bool) {
    let parameters = std::str::from_utf8(sequence).unwrap_or_default();
    for parameter in parameters
        .split(';')
        .map(|part| part.parse::<u8>().unwrap_or(0))
    {
        match parameter {
            0 => {
                *foreground = None;
                *bold = false;
            }
            1 => *bold = true,
            22 => *bold = false,
            30..=37 => *foreground = Some(parameter - 30),
            39 => *foreground = None,
            90..=97 => *foreground = Some(parameter - 90 + 8),
            _ => {}
        }
    }
}

fn terminal_color(index: Option<u8>, bold: bool) -> Option<[u8; 3]> {
    let index = index.map(|index| if bold && index < 8 { index + 8 } else { index })?;
    Some(match index {
        0 => [0, 0, 0],
        1 => [205, 49, 49],
        2 => [13, 188, 121],
        3 => [229, 229, 16],
        4 => [36, 114, 200],
        5 => [188, 63, 188],
        6 => [17, 168, 205],
        7 => [229, 229, 229],
        8 => [102, 102, 102],
        9 => [241, 76, 76],
        10 => [35, 209, 139],
        11 => [245, 245, 67],
        12 => [59, 142, 234],
        13 => [214, 112, 214],
        14 => [41, 184, 219],
        _ => [255, 255, 255],
    })
}

#[cfg(test)]
mod tests {
    use super::TerminalSession;
    use std::path::Path;

    #[cfg(unix)]
    #[test]
    fn starts_interactive_shell_in_requested_directory() {
        let mut terminal = TerminalSession::spawn(Path::new(".")).unwrap();
        terminal
            .write_input("printf 'TYIN_TERMINAL_TEST\\n'\n")
            .unwrap();
        terminal
            .write_input("printf '\\033[31mTYIN_RED_TEST\\033[0m\\n'\n")
            .unwrap();
        for _ in 0..20 {
            let output = terminal.colored_output();
            let plain_ready = output.iter().any(|line| {
                line.iter()
                    .map(|segment| segment.text.as_str())
                    .collect::<String>()
                    .trim()
                    == "TYIN_TERMINAL_TEST"
            });
            let color_ready = output.iter().any(|line| {
                line.iter()
                    .any(|segment| segment.text == "TYIN_RED_TEST" && segment.color.is_some())
            });
            if plain_ready && color_ready {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(terminal.is_running().unwrap());
        let colored = terminal.colored_output();
        assert!(
            colored.iter().any(|line| {
                line.iter()
                    .map(|segment| segment.text.as_str())
                    .collect::<String>()
                    .trim()
                    == "TYIN_TERMINAL_TEST"
            }),
            "{colored:?}"
        );
        assert!(
            colored.iter().any(|line| {
                line.iter()
                    .any(|segment| segment.text == "TYIN_RED_TEST" && segment.color.is_some())
            }),
            "{colored:?}"
        );
    }
}
