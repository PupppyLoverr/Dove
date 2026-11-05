//! Now playing, as Dove's notch had it (its notch/media.rs): what Windows'
//! media controls report (Spotify, browsers, Apple Music, YouTube Music,
//! Media Player…), with play and pause, skip, and seek. Windows'
//! `GlobalSystemMediaTransportControlsSessionManager` on a thread of its own,
//! woken only by its change events, never a timer. A track's cover is kept
//! as a picture the scripts show (`host:media/cover`).
//!
//! Scripts see `kx.media`: { app, title, artist, playing, position,
//! duration, at (unix seconds when `position` was read), cover (a picture's
//! src, or "") } or nil while nothing plays; and call `media.control` with
//! "play" | "next" | "previous" | { seek = fraction }.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::{Value as Json, json};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Value};

#[derive(Clone, Copy, Debug)]
pub enum Control {
    PlayPause,
    Next,
    Previous,
    /// To a fraction (0–1) of the track.
    Seek(f32),
}

/// What's playing, for the scripts, and its cover's pixels when new.
type Update = (Option<Json>, Option<(String, u32, u32, Vec<u8>)>);

struct Source {
    /// Each window's own line: every update goes to all of them (one
    /// queue shared would hand each update to one window only).
    windows: std::sync::Mutex<Vec<flume::Sender<Update>>>,
    /// The latest, and the latest cover, for a window that opens later.
    last: std::sync::Mutex<Update>,
    commands: flume::Sender<Control>,
}

/// One watcher a process: every window that shows music reads it.
fn source() -> &'static Source {
    static SOURCE: OnceLock<Source> = OnceLock::new();
    SOURCE.get_or_init(|| {
        let (tx, updates) = flume::unbounded::<Update>();
        let (commands, rx) = flume::unbounded();
        #[cfg(windows)]
        {
            let _ = std::thread::Builder::new().name("kx-media".into()).spawn(move || session::run(tx, rx));
        }
        #[cfg(not(windows))]
        std::mem::forget((tx, rx));
        let _ = std::thread::Builder::new().name("kx-media-out".into()).spawn(move || {
            while let Ok(update) = updates.recv() {
                let source = source();
                if let Ok(mut last) = source.last.lock() {
                    last.0 = update.0.clone();
                    if update.1.is_some() {
                        last.1 = update.1.clone();
                    }
                }
                if let Ok(mut windows) = source.windows.lock() {
                    windows.retain(|w| w.send(update.clone()).is_ok());
                }
            }
        });
        Source { windows: Default::default(), last: Default::default(), commands }
    })
}

/// A line of updates for one window, starting from what plays now.
fn subscribe() -> flume::Receiver<Update> {
    let source = source();
    let (tx, rx) = flume::unbounded();
    if let Ok(last) = source.last.lock()
        && (last.0.is_some() || last.1.is_some())
    {
        let _ = tx.send(last.clone());
    }
    if let Ok(mut windows) = source.windows.lock() {
        windows.push(tx);
    }
    rx
}

/// `kx.media` and `media.control` in this window.
pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let shared = host.shared("kx.media", Value::Nil);
    let updates = subscribe();
    let host_for_art = host.clone();
    cx.tasks.spawn(async move {
        while let Ok((now, cover)) = updates.recv_async().await {
            if let Some((name, w, h, rgba)) = cover
                && let Ok(image) = zgx::image::ImageData::new(w, h, rgba)
            {
                host_for_art.image(&name, Some(Arc::new(image)));
            }
            shared.set(now.as_ref().map(crate::panels::to_value).unwrap_or(Value::Nil));
        }
    });
    host.action("media.control", |what| {
        let control = match &what {
            Value::Str(s) if s.as_ref() == "play" => Control::PlayPause,
            Value::Str(s) if s.as_ref() == "next" => Control::Next,
            Value::Str(s) if s.as_ref() == "previous" => Control::Previous,
            other => match other.field("seek") {
                Some(Value::Num(f)) => Control::Seek(*f as f32),
                _ => return,
            },
        };
        let _ = source().commands.send(control);
    });
}

/// "Spotify.exe" or "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify" → "Spotify".
pub fn app_name(id: &str) -> String {
    let tail = id.rsplit(['!', '\\']).next().unwrap_or(id);
    let name = tail.trim_end_matches(".exe").trim_end_matches(".EXE");
    match name.to_ascii_lowercase().as_str() {
        "msedge" => "Edge".into(),
        "chrome" => "Chrome".into(),
        "firefox" => "Firefox".into(),
        "app" => "Apple Music".into(),
        "zunemusic" | "microsoft.zunemusic" => "Media Player".into(),
        _ => name.to_string(),
    }
}

fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

#[cfg(windows)]
mod session {
    use super::*;
    use windows::Foundation::TypedEventHandler;
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };
    use windows::Storage::Streams::DataReader;

    /// Whether the current track came with a cover.
    static HAS_COVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// The current session and its event registrations.
    struct Watched {
        session: Session,
        tokens: (i64, i64, i64),
    }

    impl Drop for Watched {
        fn drop(&mut self) {
            let _ = self.session.RemoveMediaPropertiesChanged(self.tokens.0);
            let _ = self.session.RemovePlaybackInfoChanged(self.tokens.1);
            let _ = self.session.RemoveTimelinePropertiesChanged(self.tokens.2);
        }
    }

    fn watch(session: Session, wake: &flume::Sender<()>) -> Option<Watched> {
        let (w1, w2, w3) = (wake.clone(), wake.clone(), wake.clone());
        let t1 = session
            .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = w1.send(());
                Ok(())
            }))
            .ok()?;
        let t2 = session
            .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                let _ = w2.send(());
                Ok(())
            }))
            .ok()?;
        let t3 = session
            .TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = w3.send(());
                Ok(())
            }))
            .ok()?;
        Some(Watched { session, tokens: (t1, t2, t3) })
    }

    pub fn run(tx: flume::Sender<Update>, rx: flume::Receiver<Control>) {
        // SAFETY: once, first thing on this thread.
        unsafe {
            let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
        }
        let manager = loop {
            match Manager::RequestAsync().and_then(|op| op.get()) {
                Ok(m) => break m,
                Err(e) => {
                    tracing::debug!("media: no session manager yet: {e}");
                    std::thread::sleep(Duration::from_secs(10));
                }
            }
        };
        let (wake_tx, wake_rx) = flume::unbounded::<()>();
        let session_wake = wake_tx.clone();
        let _ = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = session_wake.send(());
            Ok(())
        }));
        let mut watched: Option<Watched> = None;
        let mut last: Option<Json> = None;
        let mut cover_key = String::new();
        let _ = wake_tx.send(());
        loop {
            let mut control = None;
            flume::Selector::new()
                .recv(&wake_rx, |r| r.is_ok())
                .recv(&rx, |c| {
                    control = c.ok();
                    true
                })
                .wait();
            // A burst of events read once.
            while wake_rx.try_recv().is_ok() {}
            let current = manager.GetCurrentSession().ok();
            let changed = match (&watched, &current) {
                (Some(w), Some(c)) => w.session != *c,
                (None, None) => false,
                _ => true,
            };
            if changed {
                watched = current.clone().and_then(|s| watch(s, &wake_tx));
            }
            if let (Some(c), Some(s)) = (control, &current) {
                act(s, c, last.as_ref());
            }
            let (now, cover) = match current.as_ref() {
                Some(s) => read(s, &mut cover_key),
                None => (None, None),
            };
            // (The clock moving isn't a change; a seek is.)
            let same = match (&now, &last) {
                (Some(a), Some(b)) => {
                    a["title"] == b["title"]
                        && a["artist"] == b["artist"]
                        && a["playing"] == b["playing"]
                        && a["cover"] == b["cover"]
                        && (a["position"].as_f64().unwrap_or(0.) - b["position"].as_f64().unwrap_or(0.)
                            - (a["at"].as_f64().unwrap_or(0.) - b["at"].as_f64().unwrap_or(0.)))
                        .abs()
                            < 2.
                }
                (None, None) => true,
                _ => false,
            };
            if !same || cover.is_some() {
                if tx.send((now.clone(), cover)).is_err() {
                    return;
                }
                last = now;
            }
        }
    }

    fn act(s: &Session, c: Control, now: Option<&Json>) {
        let result = match c {
            Control::PlayPause => s.TryTogglePlayPauseAsync().and_then(|op| op.get()),
            Control::Next => s.TrySkipNextAsync().and_then(|op| op.get()),
            Control::Previous => s.TrySkipPreviousAsync().and_then(|op| op.get()),
            Control::Seek(fraction) => {
                let Some(m) = now else { return };
                let to = m["duration"].as_f64().unwrap_or(0.) * fraction.clamp(0., 1.) as f64;
                // 100 ns ticks.
                s.TryChangePlaybackPositionAsync((to * 10_000_000.) as i64).and_then(|op| op.get())
            }
        };
        if let Err(e) = result {
            tracing::debug!("media: {c:?} failed: {e}");
        }
    }

    fn read(s: &Session, cover_key: &mut String) -> super::Update {
        let Some(props) = s.TryGetMediaPropertiesAsync().ok().and_then(|op| op.get().ok()) else {
            return (None, None);
        };
        let title = props.Title().map(|t| t.to_string()).unwrap_or_default();
        if title.is_empty() {
            return (None, None);
        }
        let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
        let key = format!("{title}\u{1}{artist}");
        // Each track's cover its own picture, so a new one never shows an
        // old one's.
        let name = format!("media/cover-{:x}", {
            use std::hash::{Hash as _, Hasher as _};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut h);
            h.finish()
        });
        let mut cover = None;
        if *cover_key != key {
            *cover_key = key.clone();
            cover = props.Thumbnail().ok().and_then(|t| thumbnail(&t)).map(|(w, h, px)| (name.clone(), w, h, px));
            HAS_COVER.store(cover.is_some(), std::sync::atomic::Ordering::Relaxed);
        }
        let info = s.GetPlaybackInfo().ok();
        let playing = info.as_ref().and_then(|p| p.PlaybackStatus().ok()).is_some_and(|st| st == Status::Playing);
        let (position, duration) = s
            .GetTimelineProperties()
            .map(|t| {
                let pos = t.Position().map(|d| d.Duration).unwrap_or(0);
                let end = t.EndTime().map(|d| d.Duration).unwrap_or(0);
                let updated = t.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0);
                let mut pos = pos;
                if playing && updated > 0 {
                    pos += (filetime_now() - updated).max(0);
                }
                (pos.max(0) as f64 / 1e7, end.max(0) as f64 / 1e7)
            })
            .unwrap_or_default();
        let app_id = s.SourceAppUserModelId().map(|a| a.to_string()).unwrap_or_default();
        let has_cover = HAS_COVER.load(std::sync::atomic::Ordering::Relaxed);
        (
            Some(json!({
                "app": app_name(&app_id),
                "title": title,
                "artist": artist,
                "playing": playing,
                "position": if duration > 0. { position.min(duration) } else { position },
                "duration": duration,
                "at": unix_now(),
                // (The key changes with the track, so the picture reloads.)
                "cover": if has_cover { format!("host:{name}") } else { String::new() },
            })),
            cover,
        )
    }

    /// Now as a FILETIME (100 ns ticks since 1601), WinRT's DateTime.
    fn filetime_now() -> i64 {
        const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        EPOCH_DIFF + (unix.as_nanos() / 100) as i64
    }

    fn thumbnail(t: &windows::Storage::Streams::IRandomAccessStreamReference) -> Option<(u32, u32, Vec<u8>)> {
        let stream = t.OpenReadAsync().ok()?.get().ok()?;
        let size = stream.Size().ok()? as u32;
        if size == 0 || size > 8 * 1024 * 1024 {
            return None;
        }
        let reader = DataReader::CreateDataReader(&stream).ok()?;
        reader.LoadAsync(size).ok()?.get().ok()?;
        let mut bytes = vec![0u8; size as usize];
        reader.ReadBytes(&mut bytes).ok()?;
        let img = image::load_from_memory(&bytes).ok()?.thumbnail(256, 256).to_rgba8();
        let (w, h) = img.dimensions();
        Some((w, h, img.into_raw()))
    }
}

#[cfg(test)]
mod tests {
    use super::app_name;

    #[test]
    fn app_names_read_well() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "Spotify");
        assert_eq!(app_name("MSEdge"), "Edge");
    }
}
