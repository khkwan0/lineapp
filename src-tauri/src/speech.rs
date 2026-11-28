use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const MODEL_NAME: &str = "ggml-tiny-q5_1.bin";
const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin";
const MODEL_MIN_BYTES: u64 = 20_000_000;
const RECORD_SAMPLES: &str = "1440000";

struct Session {
    recorder: Recorder,
    path: PathBuf,
}

enum Recorder {
    Process(Child),
    #[cfg(target_os = "macos")]
    Core(CoreRecorder),
}

static DICTATE: std::sync::Mutex<Option<Session>> = std::sync::Mutex::new(None);

fn speech_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("speech")];
    // A packaged build installs the engine under <prefix>/share/lineapp/speech.
    // The Flatpak prefix is /app. A development binary has no such directory.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(prefix) = exe.parent().and_then(|dir| dir.parent()) {
            dirs.push(prefix.join("share").join("lineapp").join("speech"));
        }
    }
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join("lineapp").join("speech"));
    }
    dirs
}

fn find_file(name: &str) -> Option<PathBuf> {
    speech_dirs().into_iter().map(|dir| dir.join(name)).find(|path| path.is_file())
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|path| path.is_file())
}

fn engine() -> Result<PathBuf, String> {
    find_file("whisper-cli").or_else(|| on_path("whisper-cli")).ok_or_else(|| "no-engine".to_string())
}

fn file_ok(path: &Path) -> bool {
    std::fs::metadata(path).map(|meta| meta.len() >= MODEL_MIN_BYTES).unwrap_or(false)
}

fn download_model(dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "no-model".to_string())?;
    }
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(180)).build();
    let response = agent.get(MODEL_URL).call().map_err(|_| "no-model".to_string())?;
    let mut file = std::fs::File::create(dest).map_err(|_| "no-model".to_string())?;
    let mut reader = response.into_reader().take(80_000_000);
    std::io::copy(&mut reader, &mut file).map_err(|_| "no-model".to_string())?;
    file.flush().map_err(|_| "no-model".to_string())?;
    Ok(())
}

fn ensure_model() -> Result<PathBuf, String> {
    if let Some(path) = find_file(MODEL_NAME) {
        if file_ok(&path) {
            return Ok(path);
        }
    }
    let dir = dirs::data_dir().ok_or_else(|| "no-model".to_string())?.join("lineapp").join("speech");
    std::fs::create_dir_all(&dir).map_err(|_| "no-model".to_string())?;
    let dest = dir.join(MODEL_NAME);
    let part = dir.join(format!("{MODEL_NAME}.partial"));
    download_model(&part)?;
    if !file_ok(&part) {
        let _ = std::fs::remove_file(&part);
        return Err("no-model".to_string());
    }
    std::fs::rename(&part, &dest).map_err(|_| "no-model".to_string())?;
    Ok(dest)
}

fn whisper_lang(tag: &str) -> String {
    let primary = tag.split(['-', '_']).next().unwrap_or("").trim().to_ascii_lowercase();
    const KNOWN: &[&str] = &[
        "en", "ja", "ko", "zh", "th", "vi", "id", "ms", "es", "pt", "fr", "de", "it", "ru", "ar", "tr", "el", "hi",
    ];
    if KNOWN.contains(&primary.as_str()) {
        primary
    } else {
        "auto".to_string()
    }
}

fn clean_transcript(raw: &str) -> String {
    let mut parts = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.contains("blank_audio") || lower == "[silence]" || lower == "(silence)" {
            continue;
        }
        parts.push(line);
    }
    let text = parts.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
    text.chars().take(4000).collect()
}

fn command(bin: &Path) -> Command {
    let mut cmd = Command::new(bin);
    if let Some(dir) = bin.parent() {
        let libs = dir.display().to_string();
        for key in ["LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH"] {
            let joined = match std::env::var(key) {
                Ok(prev) if !prev.is_empty() => format!("{libs}:{prev}"),
                _ => libs.clone(),
            };
            cmd.env(key, joined);
        }
    }
    cmd
}

fn read_pcm(path: &Path) -> Vec<i16> {
    let data = std::fs::read(path).unwrap_or_default();
    if data.len() < 44 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Vec::new();
    }
    let mut index = 12usize;
    while index + 8 <= data.len() {
        let id = &data[index..index + 4];
        let declared = u32::from_le_bytes([data[index + 4], data[index + 5], data[index + 6], data[index + 7]]) as usize;
        let start = index + 8;
        if id == b"data" {
            let available = data.len().saturating_sub(start);
            let size = if declared == 0 || declared > available { available } else { declared };
            return data[start..start + size]
                .chunks_exact(2)
                .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
                .collect();
        }
        let step = 8 + declared + (declared % 2);
        if step == 0 {
            break;
        }
        index = index.saturating_add(step);
    }
    Vec::new()
}

fn wav_header(rate: u32, data_bytes: u32) -> [u8; 44] {
    let mut header = [0u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes());
    header[22..24].copy_from_slice(&1u16.to_le_bytes());
    header[24..28].copy_from_slice(&rate.to_le_bytes());
    header[28..32].copy_from_slice(&(rate.saturating_mul(2)).to_le_bytes());
    header[32..34].copy_from_slice(&2u16.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    header
}

fn write_wav(path: &Path, samples: &[i16]) -> Result<(), String> {
    let mut file = std::fs::File::create(path).map_err(|_| "failed".to_string())?;
    let data_bytes = (samples.len() * 2) as u32;
    file.write_all(&wav_header(16_000, data_bytes)).map_err(|_| "failed".to_string())?;
    for sample in samples {
        file.write_all(&sample.to_le_bytes()).map_err(|_| "failed".to_string())?;
    }
    Ok(())
}

fn prepare_audio(path: &Path) -> Result<PathBuf, String> {
    let samples = read_pcm(path);
    if samples.len() < 1_600 {
        return Err("empty".to_string());
    }
    let peak = samples.iter().map(|sample| sample.unsigned_abs() as i32).max().unwrap_or(0);
    if peak < 80 {
        return Err("empty".to_string());
    }
    let gain = (20_000.0 / f64::from(peak)).clamp(1.0, 40.0);
    let boosted: Vec<i16> = samples
        .iter()
        .map(|sample| (f64::from(*sample) * gain).clamp(-32_767.0, 32_767.0) as i16)
        .collect();
    let out = path.with_extension("norm.wav");
    write_wav(&out, &boosted)?;
    Ok(out)
}

pub(crate) fn level_from_peak(peak: u16) -> u32 {
    let level = (f32::from(peak) / 8_000.0).sqrt() * 100.0;
    level.round().clamp(0.0, 100.0) as u32
}

fn transcribe(wav: &Path, language: &str) -> Result<String, String> {
    let bin = engine()?;
    let model = ensure_model()?;
    let prepared = prepare_audio(wav)?;
    let threads = std::thread::available_parallelism().map(|n| n.get().clamp(1, 4)).unwrap_or(2);
    let output = command(&bin)
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(&prepared)
        .arg("-l")
        .arg(whisper_lang(language))
        .arg("-t")
        .arg(threads.to_string())
        .arg("-nt")
        .arg("-np")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let _ = std::fs::remove_file(&prepared);
    let output = output.map_err(|_| "failed".to_string())?;
    if !output.status.success() {
        return Err("failed".to_string());
    }
    let text = clean_transcript(&String::from_utf8_lossy(&output.stdout));
    if text.is_empty() {
        Err("empty".to_string())
    } else {
        Ok(text)
    }
}

fn stop_recorder(recorder: Recorder) {
    match recorder {
        Recorder::Process(child) => stop_child(child),
        #[cfg(target_os = "macos")]
        Recorder::Core(core) => stop_core(core),
    }
}

fn recorder_exited(recorder: &mut Recorder) -> bool {
    match recorder {
        Recorder::Process(child) => matches!(child.try_wait(), Ok(Some(_))),
        #[cfg(target_os = "macos")]
        Recorder::Core(core) => core.handle.as_ref().is_some_and(|handle| handle.is_finished()),
    }
}

fn stop_child(mut child: Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if started.elapsed() > Duration::from_secs(2) => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(_) => return,
        }
    }
}

fn default_source() -> String {
    #[cfg(target_os = "macos")]
    {
        use cpal::traits::{DeviceTrait, HostTrait};
        return cpal::default_host().default_input_device().and_then(|device| device.name().ok()).unwrap_or_default();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let Ok(output) = Command::new("pactl").args(["get-default-source"]).output() else {
            return String::new();
        };
        if !output.status.success() {
            return String::new();
        }
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }
}

fn spawn_record(path: &Path, source: &str) -> Result<Recorder, String> {
    #[cfg(target_os = "macos")]
    {
        return spawn_core_record(path, source, 16_000, RECORD_SAMPLES.parse::<u64>().unwrap_or(1_440_000));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut cmd = Command::new("pw-record");
        cmd.args(["--rate", "16000", "--channels", "1", "--format", "s16", "--channel-map", "mono", "--sample-count", RECORD_SAMPLES]);
        if !source.is_empty() {
            cmd.arg("--target").arg(source);
        }
        let child = cmd
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "no-mic".to_string())?;
        Ok(Recorder::Process(child))
    }
}

#[tauri::command]
pub fn dictate_start(source: String) -> Result<(), String> {
    if voice_busy() {
        return Err("failed".to_string());
    }
    {
        let slot = DICTATE.lock().expect("dictate lock");
        if slot.is_some() {
            return Err("failed".to_string());
        }
    }
    let _ = engine()?;
    let _ = ensure_model()?;
    let source = {
        let source = source.trim();
        if source.is_empty() { default_source() } else { source.to_string() }
    };
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0);
    let path = std::env::temp_dir().join(format!("lineapp-dictate-{}-{stamp}.wav", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut recorder = spawn_record(&path, &source)?;
    std::thread::sleep(Duration::from_millis(160));
    if recorder_exited(&mut recorder) {
        stop_recorder(recorder);
        let _ = std::fs::remove_file(&path);
        return Err("no-mic".to_string());
    }
    let mut slot = DICTATE.lock().expect("dictate lock");
    if slot.is_some() {
        stop_recorder(recorder);
        let _ = std::fs::remove_file(&path);
        return Err("failed".to_string());
    }
    *slot = Some(Session { recorder, path });
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicSample {
    text: String,
    audio: String,
}

fn take_recording() -> Result<Session, String> {
    DICTATE.lock().expect("dictate lock").take().ok_or_else(|| "failed".to_string())
}

#[tauri::command]
pub fn dictate_stop(language: String) -> Result<String, String> {
    let session = take_recording()?;
    stop_recorder(session.recorder);
    let result = transcribe(&session.path, &language);
    let _ = std::fs::remove_file(&session.path);
    result
}

#[tauri::command]
pub fn mic_test_stop(language: String) -> Result<MicSample, String> {
    let session = take_recording()?;
    stop_recorder(session.recorder);
    let bytes = std::fs::read(&session.path).unwrap_or_default();
    use base64::Engine;
    let audio = base64::engine::general_purpose::STANDARD.encode(bytes);
    let text = match transcribe(&session.path, &language) {
        Ok(text) => text,
        Err(code) if code == "empty" => String::new(),
        Err(code) => {
            let _ = std::fs::remove_file(&session.path);
            return Err(code);
        }
    };
    let _ = std::fs::remove_file(&session.path);
    Ok(MicSample { text, audio })
}

#[tauri::command]
pub fn dictate_cancel() {
    let session = DICTATE.lock().expect("dictate lock").take();
    if let Some(session) = session {
        stop_recorder(session.recorder);
        let _ = std::fs::remove_file(session.path);
    }
}

#[tauri::command]
pub fn dictate_level() -> u32 {
    let path = {
        let slot = DICTATE.lock().expect("dictate lock");
        let Some(session) = slot.as_ref() else {
            return 0;
        };
        session.path.clone()
    };
    let samples = read_pcm(&path);
    let peak = samples.iter().rev().take(3_200).map(|sample| sample.unsigned_abs()).max().unwrap_or(0);
    level_from_peak(peak)
}

const VOICE_RATE: u32 = 22_050;
const VOICE_BARS: usize = 28;
const VOICE_MAX_SAMPLES: u64 = VOICE_RATE as u64 * 300;

static VOICE: std::sync::Mutex<Option<Session>> = std::sync::Mutex::new(None);

pub(crate) fn voice_busy() -> bool {
    VOICE.lock().expect("voice lock").is_some()
}

#[cfg(not(target_os = "macos"))]
fn ffmpeg() -> Result<PathBuf, String> {
    on_path("ffmpeg").ok_or_else(|| "no-ffmpeg".to_string())
}

fn wav_pcm(path: &Path) -> Result<(u32, Vec<i16>), String> {
    let data = std::fs::read(path).map_err(|_| "failed".to_string())?;
    if data.len() < 44 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("failed".to_string());
    }
    let mut index = 12usize;
    let mut rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut pcm = Vec::new();
    while index + 8 <= data.len() {
        let id = &data[index..index + 4];
        let declared = u32::from_le_bytes([data[index + 4], data[index + 5], data[index + 6], data[index + 7]]) as usize;
        let start = index + 8;
        if id == b"fmt " && declared >= 16 && start + 16 <= data.len() {
            channels = u16::from_le_bytes([data[start + 2], data[start + 3]]);
            rate = u32::from_le_bytes([data[start + 4], data[start + 5], data[start + 6], data[start + 7]]);
            bits = u16::from_le_bytes([data[start + 14], data[start + 15]]);
        }
        if id == b"data" {
            let available = data.len().saturating_sub(start);
            let size = if declared == 0 || declared > available { available } else { declared };
            if bits != 16 || channels == 0 {
                return Err("failed".to_string());
            }
            let frame = usize::from(channels) * 2;
            let frames = data[start..start + size].chunks_exact(frame);
            pcm = frames.map(|frame| i16::from_le_bytes([frame[0], frame[1]])).collect();
            break;
        }
        let step = 8 + declared + (declared % 2);
        if step == 0 {
            break;
        }
        index = index.saturating_add(step);
    }
    if rate == 0 || pcm.is_empty() {
        return Err("empty".to_string());
    }
    Ok((rate, pcm))
}

fn voice_peaks(samples: &[i16]) -> Vec<u8> {
    if samples.is_empty() {
        return vec![18; VOICE_BARS];
    }
    let chunk = (samples.len() / VOICE_BARS).max(1);
    let mut raw = Vec::with_capacity(VOICE_BARS);
    let mut loudest = 1u32;
    for bar in 0..VOICE_BARS {
        let start = (bar * chunk).min(samples.len());
        let end = if bar + 1 == VOICE_BARS { samples.len() } else { (start + chunk).min(samples.len()) };
        let peak = samples[start..end].iter().map(|sample| u32::from(sample.unsigned_abs())).max().unwrap_or(0);
        loudest = loudest.max(peak);
        raw.push(peak);
    }
    raw.into_iter().map(|peak| (peak.saturating_mul(100) / loudest).clamp(14, 100) as u8).collect()
}

fn encode_voice(wav: &Path) -> Result<(Vec<u8>, i64, Vec<u8>), String> {
    #[cfg(target_os = "macos")]
    {
        return encode_voice_afconvert(wav);
    }
    #[cfg(not(target_os = "macos"))]
    {
        encode_voice_ffmpeg(wav)
    }
}

fn voice_shape(wav: &Path) -> Result<(i64, Vec<u8>, PathBuf), String> {
    let (rate, samples) = wav_pcm(wav)?;
    if samples.len() < rate as usize / 4 {
        return Err("short".to_string());
    }
    let duration = (samples.len() as i64 * 1000 / i64::from(rate)).clamp(1, 300_000);
    let peaks = voice_peaks(&samples);
    Ok((duration, peaks, wav.with_extension("m4a")))
}

fn voice_file(out: &Path, ok: bool, duration: i64, peaks: Vec<u8>) -> Result<(Vec<u8>, i64, Vec<u8>), String> {
    let bytes = if ok { std::fs::read(out).unwrap_or_default() } else { Vec::new() };
    let _ = std::fs::remove_file(out);
    if bytes.len() < 32 || &bytes[4..8] != b"ftyp" {
        return Err("failed".to_string());
    }
    Ok((bytes, duration, peaks))
}

#[cfg(target_os = "macos")]
fn encode_voice_afconvert(wav: &Path) -> Result<(Vec<u8>, i64, Vec<u8>), String> {
    let (duration, peaks, out) = voice_shape(wav)?;
    let _ = std::fs::remove_file(&out);
    let encoded = Command::new("/usr/bin/afconvert")
        .args(["-f", "m4af", "-d", "aac@22050", "-c", "1", "-b", "48000"])
        .arg(wav)
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    voice_file(&out, encoded.map(|status| status.success()).unwrap_or(false), duration, peaks)
}

#[cfg(not(target_os = "macos"))]
fn encode_voice_ffmpeg(wav: &Path) -> Result<(Vec<u8>, i64, Vec<u8>), String> {
    let bin = ffmpeg()?;
    let (duration, peaks, out) = voice_shape(wav)?;
    let _ = std::fs::remove_file(&out);
    let encoded = Command::new(&bin)
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
        ])
        .arg(wav)
        .args(["-vn", "-c:a", "aac", "-profile:a", "aac_low", "-b:a", "48k", "-ac", "1", "-ar", "22050", "-f", "ipod"])
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|_| "failed".to_string())?;
    voice_file(&out, encoded.status.success(), duration, peaks)
}

fn tail_level(path: &Path) -> u32 {
    let Ok(file) = std::fs::File::open(path) else {
        return 0;
    };
    let Ok(meta) = file.metadata() else {
        return 0;
    };
    let len = meta.len();
    if len <= 44 {
        return 0;
    }
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file;
    let take = 3_200u64 * 2;
    let start = len.saturating_sub(take).max(44);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return 0;
    }
    let mut data = Vec::new();
    if file.take(take).read_to_end(&mut data).is_err() {
        return 0;
    }
    let peak = data.chunks_exact(2).map(|sample| i16::from_le_bytes([sample[0], sample[1]]).unsigned_abs()).max().unwrap_or(0);
    level_from_peak(peak)
}

fn spawn_voice(path: &Path, source: &str) -> Result<Recorder, String> {
    #[cfg(target_os = "macos")]
    {
        return spawn_core_record(path, source, VOICE_RATE, VOICE_MAX_SAMPLES);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let samples = VOICE_MAX_SAMPLES.to_string();
        let mut cmd = Command::new("pw-record");
        cmd.args(["--rate", "22050", "--channels", "1", "--format", "s16", "--channel-map", "mono", "--sample-count", &samples]);
        if !source.is_empty() {
            cmd.arg("--target").arg(source);
        }
        let child = cmd.arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_| "no-mic".to_string())?;
        Ok(Recorder::Process(child))
    }
}

#[tauri::command]
pub fn voice_start(source: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if !Path::new("/usr/bin/afconvert").is_file() {
        return Err("no-ffmpeg".to_string());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = ffmpeg()?;
    }
    {
        if DICTATE.lock().expect("dictate lock").is_some() {
            return Err("busy".to_string());
        }
        if VOICE.lock().expect("voice lock").is_some() {
            return Err("busy".to_string());
        }
    }
    let source = {
        let source = source.trim();
        if source.is_empty() { default_source() } else { source.to_string() }
    };
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0);
    let path = std::env::temp_dir().join(format!("lineapp-voice-{}-{stamp}.wav", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut recorder = spawn_voice(&path, &source)?;
    std::thread::sleep(Duration::from_millis(80));
    if recorder_exited(&mut recorder) {
        stop_recorder(recorder);
        let _ = std::fs::remove_file(&path);
        return Err("no-mic".to_string());
    }
    let mut slot = VOICE.lock().expect("voice lock");
    if slot.is_some() {
        stop_recorder(recorder);
        let _ = std::fs::remove_file(&path);
        return Err("busy".to_string());
    }
    *slot = Some(Session { recorder, path });
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceClip {
    data: String,
    mime: String,
    duration: i64,
    peaks: Vec<u8>,
}

#[tauri::command]
pub fn voice_stop() -> Result<VoiceClip, String> {
    let session = VOICE.lock().expect("voice lock").take().ok_or_else(|| "failed".to_string())?;
    stop_recorder(session.recorder);
    let encoded = encode_voice(&session.path);
    let _ = std::fs::remove_file(&session.path);
    let (bytes, duration, peaks) = encoded?;
    use base64::Engine;
    Ok(VoiceClip {
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        mime: "audio/mp4".into(),
        duration,
        peaks,
    })
}

#[tauri::command]
pub fn voice_cancel() {
    let session = VOICE.lock().expect("voice lock").take();
    if let Some(session) = session {
        stop_recorder(session.recorder);
        let _ = std::fs::remove_file(session.path);
    }
}

#[tauri::command]
pub fn voice_level() -> u32 {
    let path = {
        let slot = VOICE.lock().expect("voice lock");
        let Some(session) = slot.as_ref() else {
            return 0;
        };
        session.path.clone()
    };
    tail_level(&path)
}

#[cfg(target_os = "macos")]
struct CoreRecorder {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(any(test, target_os = "macos"))]
struct Resampler {
    step: f64,
    phase: f64,
    held: Vec<f32>,
}

#[cfg(any(test, target_os = "macos"))]
impl Resampler {
    fn new(input_rate: u32, output_rate: u32) -> Self {
        let step = f64::from(input_rate.max(1)) / f64::from(output_rate.max(1));
        Self { step, phase: 0.0, held: Vec::new() }
    }

    fn push(&mut self, frames: &[f32]) -> Vec<i16> {
        self.held.extend_from_slice(frames);
        let mut out = Vec::new();
        while self.phase + 1.0 < self.held.len() as f64 {
            let index = self.phase as usize;
            let frac = (self.phase - index as f64) as f32;
            let sample = self.held[index] * (1.0 - frac) + self.held[index + 1] * frac;
            out.push((sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16);
            self.phase += self.step;
        }
        let drop_count = (self.phase as usize).min(self.held.len());
        if drop_count > 0 {
            self.held.drain(..drop_count);
            self.phase -= drop_count as f64;
        }
        out
    }
}

#[cfg(target_os = "macos")]
fn spawn_core_record(path: &Path, source: &str, rate: u32, max_samples: u64) -> Result<Recorder, String> {
    let device = core_input(source)?;
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    let path = path.to_path_buf();
    let handle = std::thread::spawn(move || capture_wav(device, &path, rate, max_samples, &flag));
    Ok(Recorder::Core(CoreRecorder { stop, handle: Some(handle) }))
}

#[cfg(target_os = "macos")]
fn stop_core(mut core: CoreRecorder) {
    use std::sync::atomic::Ordering;
    core.stop.store(true, Ordering::Relaxed);
    if let Some(handle) = core.handle.take() {
        let _ = handle.join();
    }
}

#[cfg(target_os = "macos")]
fn core_input(source: &str) -> Result<cpal::Device, String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    if source.is_empty() {
        return host.default_input_device().ok_or_else(|| "no-mic".to_string());
    }
    host.input_devices()
        .map_err(|_| "no-mic".to_string())?
        .find(|device| device.name().ok().as_deref() == Some(source))
        .ok_or_else(|| "no-mic".to_string())
}

#[cfg(target_os = "macos")]
fn capture_wav(device: cpal::Device, path: &Path, rate: u32, max_samples: u64, stop: &std::sync::atomic::AtomicBool) {
    use std::io::{Seek, SeekFrom, Write};
    use std::sync::atomic::Ordering;
    use cpal::traits::{DeviceTrait, StreamTrait};

    let Ok(supported) = device.default_input_config() else {
        return;
    };
    let input_rate = supported.sample_rate().0;
    let channels = supported.channels();
    let format = supported.sample_format();
    let stream_config = supported.config();
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<i16>>(32);
    let stream = match format {
        cpal::SampleFormat::F32 => open_input::<f32>(&device, &stream_config, channels, input_rate, rate, tx),
        cpal::SampleFormat::I16 => open_input::<i16>(&device, &stream_config, channels, input_rate, rate, tx),
        cpal::SampleFormat::U16 => open_input::<u16>(&device, &stream_config, channels, input_rate, rate, tx),
        cpal::SampleFormat::F64 => open_input::<f64>(&device, &stream_config, channels, input_rate, rate, tx),
        _ => return,
    };
    let Ok(stream) = stream else {
        return;
    };
    if stream.play().is_err() {
        return;
    }
    let Ok(mut file) = std::fs::File::create(path) else {
        return;
    };
    if file.write_all(&wav_header(rate, 0)).is_err() {
        return;
    }
    let mut written = 0u64;
    while !stop.load(Ordering::Relaxed) && written < max_samples {
        match rx.recv_timeout(Duration::from_millis(40)) {
            Ok(samples) => written += write_samples(&mut file, &samples, max_samples.saturating_sub(written)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(stream);
    while written < max_samples {
        let Ok(samples) = rx.try_recv() else {
            break;
        };
        written += write_samples(&mut file, &samples, max_samples.saturating_sub(written));
    }
    let data_bytes = (written.min(u64::from(u32::MAX) / 2) * 2) as u32;
    if file.seek(SeekFrom::Start(0)).is_ok() {
        let _ = file.write_all(&wav_header(rate, data_bytes));
    }
}

#[cfg(target_os = "macos")]
fn write_samples(file: &mut std::fs::File, samples: &[i16], room: u64) -> u64 {
    use std::io::Write;
    let room = usize::try_from(room).unwrap_or(usize::MAX).min(samples.len());
    if room == 0 {
        return 0;
    }
    let mut bytes = Vec::with_capacity(room * 2);
    for sample in &samples[..room] {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    if file.write_all(&bytes).is_err() {
        return 0;
    }
    let _ = file.flush();
    room as u64
}

#[cfg(target_os = "macos")]
fn open_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: u16,
    input_rate: u32,
    output_rate: u32,
    tx: std::sync::mpsc::SyncSender<Vec<i16>>,
) -> Result<cpal::Stream, ()>
where
    T: cpal::SizedSample,
{
    use cpal::traits::DeviceTrait;
    let mut resampler = Resampler::new(input_rate, output_rate);
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data, _| {
                let frames = mono_frames(data, channels);
                let samples = resampler.push(&frames);
                if !samples.is_empty() {
                    let _ = tx.try_send(samples);
                }
            },
            |_| {},
            None,
        )
        .map_err(|_| ())
}

#[cfg(target_os = "macos")]
fn mono_frames<T: cpal::Sample>(data: &[T], channels: u16) -> Vec<f32> {
    use cpal::Sample;
    let channels = usize::from(channels.max(1));
    data.chunks(channels)
        .map(|frame| {
            let sum: f32 = frame.iter().map(|sample| sample.to_sample::<f32>()).sum();
            sum / channels as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::level_from_peak;

    #[test]
    fn quiet_room_barely_moves_the_meter() {
        assert!(level_from_peak(200) < 22);
        assert!(level_from_peak(2_000) > 40);
    }

    #[test]
    fn microphone_audio_is_resampled_to_the_recording_rate() {
        let mut resampler = super::Resampler::new(48_000, 16_000);
        let frames: Vec<f32> = (0..48_000).map(|index| if index % 2 == 0 { 0.5 } else { -0.5 }).collect();
        let samples = resampler.push(&frames);
        assert!(samples.len() > 15_000 && samples.len() < 17_000, "{}", samples.len());
    }

    #[test]
    fn a_growing_recording_can_be_metered_before_the_header_is_finished() {
        let path = std::env::temp_dir().join(format!("lineapp-wav-test-{}", std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        use std::io::Write;
        file.write_all(&super::wav_header(16_000, 0)).unwrap();
        file.write_all(&1_000i16.to_le_bytes()).unwrap();
        file.write_all(&0i16.to_le_bytes()).unwrap();
        drop(file);
        let samples = super::read_pcm(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(samples, vec![1_000, 0]);
    }

    #[test]
    fn a_voice_note_is_aac_in_an_m4a() {
        let path = std::env::temp_dir().join(format!("lineapp-voice-test-{}.wav", std::process::id()));
        let samples: Vec<i16> = (0..16_000).map(|index| ((((index as f32) * 0.08).sin()) * 8000.0) as i16).collect();
        super::write_wav(&path, &samples).unwrap();
        let encoded = super::encode_voice(&path);
        let _ = std::fs::remove_file(&path);
        let (bytes, duration, peaks) = encoded.expect("ffmpeg");
        assert!(bytes.len() > 32 && &bytes[4..8] == b"ftyp");
        assert!((800..1_300).contains(&duration), "{duration}");
        assert_eq!(peaks.len(), 28);
    }
}
