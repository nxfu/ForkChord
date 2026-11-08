//! souvlaki 0.8.3 with the exact signatures its sources declare (linux/mod.rs,
//! macos/mod.rs, windows/mod.rs, lib.rs, config.rs — checked against the
//! downloaded crate). The real crate needs libdbus for its Linux MPRIS branch,
//! which this host has no system library for; the point here is the shell's
//! own code compiling against the API.

#![allow(dead_code)]

use std::time::Duration;

#[derive(Debug)]
pub struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub struct PlatformConfig {
    pub dbus_name: &'static str,
    pub display_name: &'static str,
    pub hwnd: Option<*mut std::ffi::c_void>,
}

pub enum MediaPlayback {
    Stopped,
    Paused { progress: Option<MediaPosition> },
    Playing { progress: Option<MediaPosition> },
}

pub struct MediaMetadata<'a> {
    pub title: Option<&'a str>,
    pub album: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub cover_url: Option<&'a str>,
    pub duration: Option<Duration>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MediaPosition(pub Duration);

#[derive(Clone, PartialEq, Debug)]
pub enum MediaControlEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    Seek(SeekDirection),
    SeekBy(SeekDirection, Duration),
    SetPosition(MediaPosition),
    Quit,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SeekDirection {
    Forward,
    Backward,
}

pub struct MediaControls;

impl MediaControls {
    pub fn new(config: PlatformConfig) -> Result<Self, Error> {
        let _ = config;
        Ok(Self)
    }

    pub fn attach<F>(&mut self, event_handler: F) -> Result<(), Error>
    where
        F: Fn(MediaControlEvent) + Send + 'static,
    {
        let _ = event_handler;
        Ok(())
    }

    pub fn detach(&mut self) -> Result<(), Error> {
        Ok(())
    }

    pub fn set_playback(&mut self, playback: MediaPlayback) -> Result<(), Error> {
        let _ = playback;
        Ok(())
    }

    pub fn set_metadata(&mut self, metadata: MediaMetadata<'_>) -> Result<(), Error> {
        let _ = metadata;
        Ok(())
    }
}
