//! Now Playing: what the system's media controls report (Spotify, browsers,
//! Apple Music, YouTube Music…), with play/pause, skip, seek and shuffle.
//!
//! On Windows this is `GlobalSystemMediaTransportControlsSessionManager` on a
//! thread of its own, woken only by its change events (the session, its
//! track, playback state, timeline), never on a timer. Each new track's art
//! is decoded once, with its dominant colour (the island's glow) and a
//! blurred copy (the player's backdrop). Other platforms report nothing yet.

use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Media {
    pub app: String,
    /// The player's app id, for reopening it.
    pub app_id: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub position: Duration,
    pub duration: Duration,
    /// When `position` was read; it advances from here while playing.
    pub at: Instant,
    pub art: Option<Arc<gpui::Image>>,
    /// The art blurred and darkened, for the player card's backdrop (BGRA).
    pub backdrop: Option<Arc<gpui::RenderImage>>,
    /// The art's dominant colour (r, g, b), for the glass glow.
    pub tint: Option<(u8, u8, u8)>,
    pub can_seek: bool,
    pub shuffle: Option<bool>,
}

impl PartialEq for Media {
    fn eq(&self, other: &Self) -> bool {
        self.app_id == other.app_id
            && self.title == other.title
            && self.artist == other.artist
            && self.playing == other.playing
            && self.duration == other.duration
            && self.shuffle == other.shuffle
            && same(&self.art, &other.art)
            // A seek, not the clock moving.
            && self.now().abs_diff(other.now()) < Duration::from_secs(2)
    }
}

fn same<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

impl Media {
    /// The playback position now.
    pub fn now(&self) -> Duration {
        let p = if self.playing { self.position + self.at.elapsed() } else { self.position };
        if self.duration.is_zero() { p } else { p.min(self.duration) }
    }

    pub fn progress(&self) -> f32 {
        if self.duration.is_zero() { 0.0 } else { self.now().as_secs_f32() / self.duration.as_secs_f32() }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Control {
    PlayPause,
    Next,
    Previous,
    /// To a fraction (0–1) of the track.
    Seek(f32),
    Shuffle(bool),
}

pub struct Source {
    pub updates: flume::Receiver<Option<Media>>,
    commands: flume::Sender<Control>,
}

impl Source {
    pub fn send(&self, c: Control) {
        let _ = self.commands.send(c);
    }
}

pub fn start() -> Source {
    let (tx, updates) = flume::unbounded();
    let (commands, rx) = flume::unbounded();
    #[cfg(windows)]
    {
        let _ = std::thread::Builder::new().name("island-media".into()).spawn(move || windows::run(tx, rx));
    }
    #[cfg(not(windows))]
    {
        std::mem::forget((tx, rx));
    }
    Source { updates, commands }
}

/// Opens a player the island has seen before (Spotify, Music, a browser).
pub fn open_player(app_id: &str) {
    #[cfg(windows)]
    {
        let target = if app_id.eq_ignore_ascii_case("spotify.exe") {
            "spotify:".to_string()
        } else {
            format!("shell:AppsFolder\\{app_id}")
        };
        let _ = std::process::Command::new("explorer.exe").arg(target).spawn();
    }
    #[cfg(not(windows))]
    let _ = app_id;
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

/// The average colour of the art's mid tones (skipping near-black and
/// near-white), as the glass glow.
pub fn dominant(img: &image::RgbaImage) -> Option<(u8, u8, u8)> {
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for p in img.pixels() {
        let [pr, pg, pb, _] = p.0;
        let max = pr.max(pg).max(pb) as u32;
        let min = pr.min(pg).min(pb) as u32;
        if max < 40 || min > 225 {
            continue;
        }
        // Saturated pixels count more: they're the art's colour.
        let w = 1 + (max - min) as u64 / 16;
        r += pr as u64 * w;
        g += pg as u64 * w;
        b += pb as u64 * w;
        n += w;
    }
    (n > 0).then(|| ((r / n) as u8, (g / n) as u8, (b / n) as u8))
}

/// The art, small, blurred and darkened, in BGRA for GPUI.
pub fn backdrop_of(img: &image::RgbaImage) -> Arc<gpui::RenderImage> {
    let small = image::imageops::resize(img, 48, 48, image::imageops::FilterType::Triangle);
    let mut blurred = image::imageops::blur(&small, 6.0);
    for p in blurred.pixels_mut() {
        let [r, g, b, _] = p.0;
        p.0 = [(b as f32 * 0.55) as u8, (g as f32 * 0.55) as u8, (r as f32 * 0.55) as u8, 255];
    }
    Arc::new(gpui::RenderImage::new([image::Frame::new(blurred)]))
}

#[cfg(windows)]
mod windows {
    use super::*;
    use ::windows::Foundation::TypedEventHandler;
    use ::windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };
    use ::windows::Storage::Streams::DataReader;

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
        let w1 = wake.clone();
        let w2 = wake.clone();
        let w3 = wake.clone();
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

    pub fn run(tx: flume::Sender<Option<Media>>, rx: flume::Receiver<Control>) {
        // SAFETY: once, first thing on this thread.
        unsafe {
            let _ = ::windows::Win32::System::WinRT::RoInitialize(::windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
        }
        let manager = loop {
            match Manager::RequestAsync().and_then(|op| op.get()) {
                Ok(m) => break m,
                Err(e) => {
                    tracing::debug!("notch media: no session manager yet: {e}");
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
        let mut last: Option<Media> = None;
        let mut art_key = String::new();
        let mut art: (Option<Arc<gpui::Image>>, Option<Arc<gpui::RenderImage>>, Option<(u8, u8, u8)>) = (None, None, None);
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
            // Coalesce a burst of events into one read.
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
                // Players report the change through their events.
            }
            let now = current.as_ref().and_then(|s| read(s, &mut art_key, &mut art));
            if now != last {
                if tx.send(now.clone()).is_err() {
                    return;
                }
                last = now;
            }
        }
    }

    fn act(s: &Session, c: Control, now: Option<&Media>) {
        let result = match c {
            Control::PlayPause => s.TryTogglePlayPauseAsync().and_then(|op| op.get()),
            Control::Next => s.TrySkipNextAsync().and_then(|op| op.get()),
            Control::Previous => s.TrySkipPreviousAsync().and_then(|op| op.get()),
            Control::Seek(fraction) => {
                let Some(m) = now else { return };
                let to = m.duration.mul_f32(fraction.clamp(0.0, 1.0));
                // 100 ns ticks.
                s.TryChangePlaybackPositionAsync((to.as_nanos() / 100) as i64).and_then(|op| op.get())
            }
            Control::Shuffle(on) => s.TryChangeShuffleActiveAsync(on).and_then(|op| op.get()),
        };
        if let Err(e) = result {
            tracing::debug!("notch media: {c:?} failed: {e}");
        }
    }

    fn read(
        s: &Session,
        art_key: &mut String,
        art: &mut (Option<Arc<gpui::Image>>, Option<Arc<gpui::RenderImage>>, Option<(u8, u8, u8)>),
    ) -> Option<Media> {
        let props = s.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
        let title = props.Title().ok()?.to_string();
        if title.is_empty() {
            return None;
        }
        let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
        let key = format!("{title}\u{1}{artist}");
        if *art_key != key {
            *art_key = key;
            *art = props.Thumbnail().ok().and_then(|t| thumbnail(&t)).unwrap_or_default();
        }
        let info = s.GetPlaybackInfo().ok();
        let playing = info.as_ref().and_then(|p| p.PlaybackStatus().ok()).is_some_and(|st| st == Status::Playing);
        let controls = info.as_ref().and_then(|p| p.Controls().ok());
        let can_seek = controls.as_ref().and_then(|c| c.IsPlaybackPositionEnabled().ok()).unwrap_or(false);
        let shuffle_enabled = controls.as_ref().and_then(|c| c.IsShuffleEnabled().ok()).unwrap_or(false);
        let shuffle = if shuffle_enabled {
            info.as_ref().and_then(|p| p.IsShuffleActive().ok()).and_then(|r| r.Value().ok())
        } else {
            None
        };
        let (mut position, duration) = s
            .GetTimelineProperties()
            .map(|t| {
                let pos = t.Position().map(|d| d.Duration).unwrap_or(0);
                let end = t.EndTime().map(|d| d.Duration).unwrap_or(0);
                // Timeline values are 100 ns ticks; LastUpdatedTime says when
                // the player reported the position.
                let updated = t.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0);
                let mut pos = pos;
                if playing && updated > 0 {
                    pos += (filetime_now() - updated).max(0);
                }
                (ticks(pos), ticks(end))
            })
            .unwrap_or_default();
        if !duration.is_zero() {
            position = position.min(duration);
        }
        let app_id = s.SourceAppUserModelId().map(|a| a.to_string()).unwrap_or_default();
        Some(Media {
            app: app_name(&app_id),
            app_id,
            title,
            artist,
            playing,
            position,
            duration,
            at: Instant::now(),
            art: art.0.clone(),
            backdrop: art.1.clone(),
            tint: art.2,
            can_seek,
            shuffle,
        })
    }

    fn ticks(t: i64) -> Duration {
        Duration::from_nanos((t.max(0) as u64).saturating_mul(100))
    }

    /// Now as a FILETIME (100 ns ticks since 1601), WinRT's DateTime.
    fn filetime_now() -> i64 {
        const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        EPOCH_DIFF + (unix.as_nanos() / 100) as i64
    }

    #[allow(clippy::type_complexity)]
    fn thumbnail(
        t: &::windows::Storage::Streams::IRandomAccessStreamReference,
    ) -> Option<(Option<Arc<gpui::Image>>, Option<Arc<gpui::RenderImage>>, Option<(u8, u8, u8)>)> {
        let stream = t.OpenReadAsync().ok()?.get().ok()?;
        let size = stream.Size().ok()? as u32;
        if size == 0 || size > 8 * 1024 * 1024 {
            return None;
        }
        let reader = DataReader::CreateDataReader(&stream).ok()?;
        reader.LoadAsync(size).ok()?.get().ok()?;
        let mut bytes = vec![0u8; size as usize];
        reader.ReadBytes(&mut bytes).ok()?;
        let guessed = image::guess_format(&bytes).ok()?;
        let format = match guessed {
            image::ImageFormat::Png => gpui::ImageFormat::Png,
            image::ImageFormat::Jpeg => gpui::ImageFormat::Jpeg,
            image::ImageFormat::WebP => gpui::ImageFormat::Webp,
            image::ImageFormat::Bmp => gpui::ImageFormat::Bmp,
            image::ImageFormat::Gif => gpui::ImageFormat::Gif,
            _ => return None,
        };
        let decoded = image::load_from_memory_with_format(&bytes, guessed).ok().map(|d| d.to_rgba8());
        let tint = decoded.as_ref().and_then(dominant);
        let backdrop = decoded.as_ref().map(backdrop_of);
        Some((Some(Arc::new(gpui::Image::from_bytes(format, bytes))), backdrop, tint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names_read_well() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "Spotify");
        assert_eq!(app_name("msedge"), "Edge");
    }

    #[test]
    fn the_dominant_colour_skips_black_and_white() {
        let mut img = image::RgbaImage::new(4, 4);
        for (i, p) in img.pixels_mut().enumerate() {
            p.0 = match i % 3 {
                0 => [0, 0, 0, 255],
                1 => [255, 255, 255, 255],
                _ => [200, 40, 40, 255],
            };
        }
        assert_eq!(dominant(&img), Some((200, 40, 40)));
    }
}
