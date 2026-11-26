//! `page.recordStart` / `page.recordStop` (ADR-030): the extension streams the tab's screencast
//! as JPEG frames (`ExtensionSession.recordFrame`); the Bridge paces them to a constant frame rate
//! and pipes them into ffmpeg (agent-browser `build_ffmpeg_command`, input `mjpeg`). One recording
//! at a time per Bridge.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::{mpsc, oneshot};
use vexdom_core::result::ActionError;

use crate::contact_sheet::{
    ContactSheetCollector, DEFAULT_CONTACT_SHEET_THRESHOLD, contact_sheet_path, write_contact_sheet,
};

const HIGH_FPS_THRESHOLD: u32 = 30;
const WEBM_BITRATE_KBPS: u32 = 8000;
const FINISH_TIMEOUT: Duration = Duration::from_secs(10);

fn error(code: &str, message: impl Into<String>) -> ActionError {
    ActionError {
        code: code.into(),
        message: message.into(),
    }
}

/// `ffmpeg` on the `PATH`.
pub(crate) fn find_ffmpeg() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("ffmpeg"))
        .find(|candidate| candidate.is_file())
}

fn ffmpeg_command(ffmpeg: &Path, output: &Path, fps: u32) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(ffmpeg);
    let rate = fps.to_string();
    cmd.args(["-y", "-hide_banner", "-loglevel", "error"])
        .args([
            "-fpsprobesize",
            "0",
            "-probesize",
            "32",
            "-analyzeduration",
            "0",
        ])
        .args(["-f", "image2pipe", "-c:v", "mjpeg", "-framerate", &rate])
        .args(["-i", "pipe:0"])
        .args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"]);
    if output.extension().and_then(|ext| ext.to_str()) == Some("webm") {
        cmd.args(["-c:v", "libvpx", "-crf", "18"])
            .args(["-b:v", &format!("{WEBM_BITRATE_KBPS}k")])
            .args(["-deadline", "realtime", "-cpu-used", "4"]);
    } else {
        cmd.args(["-c:v", "libx264", "-preset", "ultrafast"]);
    }
    cmd.args(["-pix_fmt", "yuv420p"])
        .args(["-threads", if fps > HIGH_FPS_THRESHOLD { "4" } else { "1" }])
        .arg(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

/// How many times the previous frame repeats before a frame that arrived `gap_ms` after it.
pub(crate) fn repeats(gap_ms: f64, fps: u32) -> u64 {
    let slots = (gap_ms.max(0.0) * f64::from(fps) / 1000.0).round();
    (slots as u64).saturating_sub(1)
}

struct Active {
    id: String,
    frames: mpsc::UnboundedSender<(Vec<u8>, f64)>,
    done: oneshot::Receiver<Result<Value, ActionError>>,
}

/// The Bridge's one video recording.
#[derive(Default)]
pub(crate) struct Video {
    active: Option<Active>,
    counter: u64,
}

pub(crate) struct Start {
    pub path: PathBuf,
    pub fps: u32,
    pub contact_sheet: Option<f64>,
}

impl Video {
    /// Starts ffmpeg and answers the recording's id.
    pub fn start(&mut self, start: Start) -> Result<String, ActionError> {
        if self.active.is_some() {
            return Err(error(
                "RECORDING_ACTIVE",
                "A recording is already running; stop it first.",
            ));
        }
        let ffmpeg = find_ffmpeg()
            .ok_or_else(|| error("FFMPEG_MISSING", "install it with brew install ffmpeg"))?;
        if let Some(parent) = start.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                error(
                    "ACTION_FAILED",
                    format!("Could not create {}: {e}", parent.display()),
                )
            })?;
        }
        let mut child = ffmpeg_command(&ffmpeg, &start.path, start.fps)
            .spawn()
            .map_err(|e| error("FFMPEG_MISSING", format!("ffmpeg did not start: {e}")))?;
        let stdin = child.stdin.take();
        self.counter += 1;
        let id = format!("rec-{}-{}", std::process::id(), self.counter);
        let (frames, receiver) = mpsc::unbounded_channel();
        let (finished, done) = oneshot::channel();
        tokio::task::spawn_local(encode(child, stdin, receiver, start, finished));
        self.active = Some(Active {
            id: id.clone(),
            frames,
            done,
        });
        Ok(id)
    }

    pub fn frame(&mut self, id: &str, bytes: &[u8], at_ms: f64) {
        if let Some(active) = self.active.as_ref().filter(|active| active.id == id) {
            let _ = active.frames.send((bytes.to_vec(), at_ms));
        }
    }

    /// Ends the recording; the returned future resolves once the file is closed.
    pub fn stop(&mut self) -> Result<oneshot::Receiver<Result<Value, ActionError>>, ActionError> {
        let active = self
            .active
            .take()
            .ok_or_else(|| error("NO_RECORDING", "No recording is running."))?;
        drop(active.frames);
        Ok(active.done)
    }

    /// Drops a recording whose start the extension refused.
    pub fn abandon(&mut self, id: &str) {
        if self.active.as_ref().is_some_and(|active| active.id == id) {
            self.active = None;
        }
    }
}

async fn encode(
    mut child: tokio::process::Child,
    mut stdin: Option<tokio::process::ChildStdin>,
    mut receiver: mpsc::UnboundedReceiver<(Vec<u8>, f64)>,
    start: Start,
    finished: oneshot::Sender<Result<Value, ActionError>>,
) {
    let mut previous: Option<(Vec<u8>, f64)> = None;
    let mut first_at: Option<f64> = None;
    let mut written: u64 = 0;
    let mut sheet = start
        .contact_sheet
        .map(|threshold| ContactSheetCollector::new(threshold.max(0.0)));
    let write = async |stdin: &mut Option<tokio::process::ChildStdin>, bytes: &[u8]| {
        if let Some(pipe) = stdin.as_mut()
            && pipe.write_all(bytes).await.is_err()
        {
            *stdin = None;
        }
    };
    while let Some((frame, at)) = receiver.recv().await {
        let first = *first_at.get_or_insert(at);
        if let Some((last, last_at)) = &previous {
            for _ in 0..repeats(at - last_at, start.fps) {
                write(&mut stdin, last).await;
                written += 1;
            }
        }
        write(&mut stdin, &frame).await;
        written += 1;
        if let Some(sheet) = sheet.as_mut() {
            sheet.consider(&frame, Duration::from_millis((at - first).max(0.0) as u64));
        }
        previous = Some((frame, at));
    }
    if let Some((last, _)) = &previous {
        write(&mut stdin, last).await;
        written += 1;
    }
    drop(stdin);
    let status = tokio::time::timeout(FINISH_TIMEOUT, child.wait()).await;
    let outcome = match status {
        Ok(Ok(status)) if status.success() => {
            let duration_ms = written * 1000 / u64::from(start.fps.max(1));
            let mut result = json!({
                "path": start.path.to_string_lossy(),
                "frames": written,
                "durationMs": duration_ms,
            });
            if let Some(sheet) = sheet {
                let cells = sheet.finish();
                let sheet_path = contact_sheet_path(&start.path.to_string_lossy());
                if !cells.is_empty() && write_contact_sheet(Path::new(&sheet_path), &cells).is_ok()
                {
                    result["contactSheetPath"] = json!(sheet_path);
                    result["contactSheetFrames"] = json!(cells.len());
                }
            }
            Ok(result)
        }
        Ok(Ok(status)) => {
            let mut tail = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                use tokio::io::AsyncReadExt as _;
                let _ = stderr.read_to_string(&mut tail).await;
            }
            let tail: String = tail
                .chars()
                .rev()
                .take(400)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            Err(error(
                "ACTION_FAILED",
                format!("ffmpeg exited with {status}: {}", tail.trim()),
            ))
        }
        Ok(Err(e)) => Err(error("ACTION_FAILED", format!("ffmpeg: {e}"))),
        Err(_) => {
            let _ = child.kill().await;
            Err(error(
                "ACTION_FAILED",
                "ffmpeg did not finish writing the video within 10 s.",
            ))
        }
    };
    let _ = finished.send(outcome);
}

/// `page.recordStart`'s input as the Bridge reads it: `path` must end in .webm or .mp4.
pub(crate) fn start_of(input: &Value) -> Result<Start, ActionError> {
    let path = input
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            error(
                "INVALID_INPUT",
                "recordStart needs a \"path\" ending in .webm or .mp4",
            )
        })?;
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(error("INVALID_INPUT", "path must be absolute"));
    }
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("webm" | "mp4") => {}
        _ => {
            return Err(error(
                "INVALID_INPUT",
                "The recording path must end in .webm or .mp4",
            ));
        }
    }
    let fps = input
        .get("fps")
        .and_then(Value::as_u64)
        .unwrap_or(30)
        .clamp(1, 60) as u32;
    let contact_sheet = (input.get("contactSheet") == Some(&Value::Bool(true))).then(|| {
        input
            .get("contactSheetThreshold")
            .and_then(Value::as_f64)
            .unwrap_or(DEFAULT_CONTACT_SHEET_THRESHOLD)
    });
    Ok(Start {
        path,
        fps,
        contact_sheet,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_repeat_to_fill_the_gap() {
        assert_eq!(repeats(0.0, 30), 0);
        assert_eq!(repeats(33.3, 30), 0);
        assert_eq!(repeats(100.0, 30), 2);
        assert_eq!(repeats(1000.0, 10), 9);
        assert_eq!(repeats(-5.0, 30), 0);
    }

    #[test]
    fn a_recording_path_ends_in_webm_or_mp4() {
        assert!(start_of(&json!({"path": "/tmp/r.webm"})).is_ok());
        assert_eq!(
            start_of(&json!({"path": "/tmp/r.gif"})).err().unwrap().code,
            "INVALID_INPUT"
        );
        assert_eq!(
            start_of(&json!({"path": "r.mp4"})).err().unwrap().message,
            "path must be absolute"
        );
        let start =
            start_of(&json!({"path": "/tmp/r.mp4", "fps": 99, "contactSheet": true})).unwrap();
        assert_eq!(start.fps, 60);
        assert_eq!(start.contact_sheet, Some(DEFAULT_CONTACT_SHEET_THRESHOLD));
    }
}
