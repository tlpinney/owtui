//! Plays a recording with an external player, one at a time.

use anyhow::{bail, Result};
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Child, Command, Stdio};

pub struct Player {
    commands: Vec<(String, Vec<String>)>,
    child: Option<Child>,
    /// The transcript whose recording is playing.
    pub playing: Option<i64>,
}

impl Default for Player {
    fn default() -> Self {
        // OpenWhispr records WebM/Opus, which afplay can't decode; it's the last resort.
        let commands = [
            (
                "ffplay",
                &["-nodisp", "-autoexit", "-loglevel", "quiet"][..],
            ),
            (
                "mpv",
                &["--no-video", "--really-quiet", "--no-terminal"][..],
            ),
            ("afplay", &[][..]),
        ];
        Self::with_commands(
            commands
                .iter()
                .map(|(cmd, args)| {
                    (
                        cmd.to_string(),
                        args.iter().map(|a| a.to_string()).collect(),
                    )
                })
                .collect(),
        )
    }
}

impl Player {
    pub fn with_commands(commands: Vec<(String, Vec<String>)>) -> Self {
        Self {
            commands,
            child: None,
            playing: None,
        }
    }

    pub fn play(&mut self, path: &Path, id: i64) -> Result<()> {
        self.stop();
        for (cmd, args) in &self.commands {
            let spawned = Command::new(cmd)
                .args(args)
                .arg(path)
                // Keep the player off the terminal the TUI is drawing on.
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            match spawned {
                Ok(child) => {
                    self.child = Some(child);
                    self.playing = Some(id);
                    return Ok(());
                }
                Err(e) if e.kind() == ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            }
        }
        bail!("No audio player found. Install ffmpeg: brew install ffmpeg")
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.playing = None;
    }

    /// Notices when playback finished on its own.
    pub fn poll(&mut self) {
        if let Some(child) = &mut self.child {
            if !matches!(child.try_wait(), Ok(None)) {
                self.child = None;
                self.playing = None;
            }
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}
