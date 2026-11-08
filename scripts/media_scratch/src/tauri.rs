//! A tauri-shaped stub, carrying the exact signatures the locked tauri 2.12.5
//! exposes for the pieces media.rs and chrome.rs touch (checked against the
//! crate sources: webview_window.rs:1954 ns_window, :1966 hwnd, :2506 url,
//! :2735 cookies, app.rs run_on_main_thread).

#![allow(dead_code)]

pub type Url = url::Url;

#[derive(Clone)]
pub struct AppHandle;

impl AppHandle {
    pub fn run_on_main_thread<F: FnOnce() + Send + 'static>(&self, f: F) -> Result<(), Error> {
        f();
        Ok(())
    }
    pub fn exit(&self, _code: i32) -> Result<(), Error> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct Error(String);

pub trait Manager {
    fn try_state<T: Send + Sync + 'static>(&self) -> Option<std::sync::Arc<T>>;
    fn state<T: Send + Sync + 'static>(&self) -> std::sync::Arc<T>;
    fn get_webview_window(&self, label: &str) -> Option<WebviewWindow>;
}

impl Manager for AppHandle {
    fn try_state<T: Send + Sync + 'static>(&self) -> Option<std::sync::Arc<T>> {
        None
    }
    fn state<T: Send + Sync + 'static>(&self) -> std::sync::Arc<T> {
        unreachable!("scratch: no state")
    }
    fn get_webview_window(&self, _label: &str) -> Option<WebviewWindow> {
        None
    }
}

pub struct WebviewWindow;

#[cfg(windows)]
pub struct Hwnd(pub isize);

impl WebviewWindow {
    #[cfg(target_os = "macos")]
    pub fn ns_window(&self) -> Result<*mut std::ffi::c_void, Error> {
        Err(Error("scratch".into()))
    }

    #[cfg(windows)]
    pub fn hwnd(&self) -> Result<Hwnd, Error> {
        Err(Error("scratch".into()))
    }

    pub fn eval(&self, _script: &str) -> Result<(), Error> {
        Ok(())
    }

    pub fn url(&self) -> Result<Url, Error> {
        Err(Error("scratch".into()))
    }
}
