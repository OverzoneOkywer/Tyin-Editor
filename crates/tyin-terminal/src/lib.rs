use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

const SCROLLBACK_LINES: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSegment {
    pub text: String,
    pub color: Option<[u8; 3]>,
    pub background: Option<[u8; 3]>,
    pub bold: bool,
}

pub struct TerminalSession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    screen: Arc<Mutex<vt100::Parser>>,
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
        command.env("TERM", "xterm-256color");
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
        let screen = Arc::new(Mutex::new(vt100::Parser::new(
            std::num::NonZeroU16::new(24).expect("24 is nonzero"),
            std::num::NonZeroU16::new(80).expect("80 is nonzero"),
            SCROLLBACK_LINES,
        )));
        spawn_reader(reader, Arc::clone(&screen));

        Ok(Self {
            master: pair.master,
            writer,
            child,
            screen,
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
        let cols = cols.max(1);
        let rows = rows.max(1);
        self.master.resize(PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.screen
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal screen lock is poisoned"))?
            .set_size(
                std::num::NonZeroU16::new(rows).expect("rows is nonzero"),
                std::num::NonZeroU16::new(cols).expect("cols is nonzero"),
            );
        Ok(())
    }

    pub fn output(&self) -> Result<Vec<String>> {
        let screen = self
            .screen
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal screen lock is poisoned"))?;
        Ok(screen
            .screen()
            .contents()
            .lines()
            .map(str::to_owned)
            .collect())
    }

    pub fn colored_output(&self) -> Result<Vec<Vec<TerminalSegment>>> {
        let screen = self
            .screen
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal screen lock is poisoned"))?;
        Ok(screen_lines(&screen))
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

fn spawn_reader(mut reader: Box<dyn Read + Send>, screen: Arc<Mutex<vt100::Parser>>) {
    thread::spawn(move || {
        let mut bytes = [0; 4096];
        loop {
            let count = match reader.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            if let Ok(mut screen) = screen.lock() {
                screen.process(&bytes[..count]);
            } else {
                break;
            }
        }
    });
}

fn screen_lines(screen: &vt100::Parser) -> Vec<Vec<TerminalSegment>> {
    let terminal = screen.screen();
    let (rows, cols) = terminal.size();
    let (cursor_row, cursor_col) = terminal.cursor_position();
    (0..rows.get())
        .map(|row| {
            let mut segments: Vec<TerminalSegment> = Vec::new();
            for col in 0..cols.get() {
                let Some(cell) = terminal.cell(row, col) else {
                    continue;
                };
                let cursor = !terminal.hide_cursor() && row == cursor_row && col == cursor_col;
                let mut color = terminal_color(cell.fgcolor());
                let mut background = terminal_color(cell.bgcolor());
                if cell.inverse() || cursor {
                    std::mem::swap(&mut color, &mut background);
                }
                if cursor && color.is_none() && background.is_none() {
                    color = Some([24, 31, 42]);
                    background = Some([220, 220, 220]);
                }
                let text = if cell.has_contents() {
                    cell.contents()
                } else {
                    " "
                };
                let bold = cell.bold();
                if let Some(last) = segments.last_mut().filter(|last| {
                    last.color == color && last.background == background && last.bold == bold
                }) {
                    last.text.push_str(text);
                } else {
                    segments.push(TerminalSegment {
                        text: text.to_owned(),
                        color,
                        background,
                        bold,
                    });
                }
            }
            while segments
                .last()
                .is_some_and(|segment| segment.color.is_none() && segment.background.is_none())
            {
                let Some(last) = segments.last_mut() else {
                    break;
                };
                let trimmed = last.text.trim_end_matches(' ').len();
                last.text.truncate(trimmed);
                if last.text.is_empty() {
                    segments.pop();
                } else {
                    break;
                }
            }
            segments
        })
        .collect()
}

fn terminal_color(color: vt100::Color) -> Option<[u8; 3]> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Rgb(red, green, blue) => Some([red, green, blue]),
        vt100::Color::Idx(index @ 0..=15) => {
            let colors = [
                [0, 0, 0],
                [205, 49, 49],
                [13, 188, 121],
                [229, 229, 16],
                [36, 114, 200],
                [188, 63, 188],
                [17, 168, 205],
                [229, 229, 229],
                [102, 102, 102],
                [241, 76, 76],
                [35, 209, 139],
                [245, 245, 67],
                [59, 142, 234],
                [214, 112, 214],
                [41, 184, 219],
                [255, 255, 255],
            ];
            Some(colors[usize::from(index)])
        }
        vt100::Color::Idx(index @ 16..=231) => {
            let index = index - 16;
            let channel = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            Some([
                channel(index / 36),
                channel(index / 6 % 6),
                channel(index % 6),
            ])
        }
        vt100::Color::Idx(index @ 232..=255) => {
            let shade = 8 + (index - 232) * 10;
            Some([shade, shade, shade])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{screen_lines, TerminalSession};
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
        for _ in 0..100 {
            let output = terminal.colored_output().unwrap();
            let plain_ready = output.iter().any(|line| {
                line.iter()
                    .map(|segment| segment.text.as_str())
                    .collect::<String>()
                    .contains("TYIN_TERMINAL_TEST")
            });
            let color_ready = output.iter().any(|line| {
                line.iter().any(|segment| {
                    segment.text.contains("TYIN_RED_TEST") && segment.color == Some([205, 49, 49])
                })
            });
            if plain_ready && color_ready {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(terminal.is_running().unwrap());
        let colored = terminal.colored_output().unwrap();
        assert!(colored.iter().any(|line| {
            line.iter()
                .map(|segment| segment.text.as_str())
                .collect::<String>()
                .contains("TYIN_TERMINAL_TEST")
        }));
        assert!(colored.iter().any(|line| {
            line.iter().any(|segment| {
                segment.text.contains("TYIN_RED_TEST") && segment.color == Some([205, 49, 49])
            })
        }));
    }

    #[test]
    fn terminal_screen_applies_cursor_motion_colors_and_screen_clears() {
        let mut parser = vt100::Parser::default();
        parser.process(b"first line\r\nsecond line");
        parser.process(b"\x1b[2;1H\x1b[38;2;12;34;56mupdated");
        parser.process(b"\x1b[3;1H\x1b[38;5;196;48;2;1;2;3;1mcolor");
        let lines = screen_lines(&parser);
        assert!(lines[0]
            .iter()
            .any(|segment| segment.text.contains("first line")));
        assert!(lines[1].iter().any(|segment| {
            segment.text.contains("updated") && segment.color == Some([12, 34, 56])
        }));
        assert!(lines[2].iter().any(|segment| {
            segment.text.contains("color")
                && segment.color == Some([255, 0, 0])
                && segment.background == Some([1, 2, 3])
                && segment.bold
        }));

        parser.process(b"\x1b[2J\x1b[Hcleared");
        let lines = screen_lines(&parser);
        assert!(lines[0]
            .iter()
            .any(|segment| segment.text.contains("cleared")));
        assert!(lines[1].iter().all(|segment| {
            !segment.text.contains("second line") && !segment.text.contains("updated")
        }));
    }
}
