//! C-API `libawg` (`engines/awg-go/awg.h`): типы колбэков, таблица функций
//! и коды ошибок. Сами символы линкуются `build.rs` из `engines/awg-go/dist/`.

use std::ffi::{c_char, c_int, c_void};

/// Расшифрованный IP-пакет от сервера; буфер принадлежит ядру — копировать.
pub type TxFn = unsafe extern "C" fn(ctx: *mut c_void, pkt: *const u8, len: usize);
/// Android: защитить UDP-сокет ядра; 1 — защищён, 0 — отказ.
pub type ProtectFn = unsafe extern "C" fn(ctx: *mut c_void, fd: c_int) -> c_int;
/// Строка журнала ядра; level — `code::LOG_ERROR` / `code::LOG_VERBOSE`.
pub type LogFn = unsafe extern "C" fn(ctx: *mut c_void, level: c_int, msg: *const c_char);

/// `awg_callbacks` из `awg.h` — раскладка `#[repr(C)]`, поля в том же порядке.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AwgCallbacks {
    pub tx: Option<TxFn>,
    pub protect: Option<ProtectFn>,
    pub log: Option<LogFn>,
}

/// Таблица функций C-API — чтобы подменять библиотеку в тестах без Go.
#[derive(Clone, Copy)]
pub struct AwgApi {
    pub new: unsafe extern "C" fn(
        uapi: *const c_char,
        mtu: c_int,
        cbs: *const AwgCallbacks,
        ctx: *mut c_void,
    ) -> i32,
    pub inject: unsafe extern "C" fn(handle: i32, pkt: *const u8, len: usize) -> c_int,
    pub set_config: unsafe extern "C" fn(handle: i32, uapi: *const c_char) -> c_int,
    pub get_config: unsafe extern "C" fn(handle: i32) -> *mut c_char,
    pub free_string: unsafe extern "C" fn(s: *mut c_char),
    pub bump_sockets: unsafe extern "C" fn(handle: i32),
    pub close: unsafe extern "C" fn(handle: i32),
}

/// Загруженная библиотека libawg: [`AwgLib::linked`] — та, что прилинкована
/// `build.rs`; [`AwgLib::from_api`] — любая таблица функций (тесты).
#[derive(Clone, Copy)]
pub struct AwgLib {
    api: AwgApi,
}

impl AwgLib {
    /// libawg, прилинкованная при сборке (`engines/awg-go/dist/<os>`).
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
    pub fn linked() -> Self {
        Self {
            api: AwgApi {
                new: linked::awg_new,
                inject: linked::awg_inject,
                set_config: linked::awg_set_config,
                get_config: linked::awg_get_config,
                free_string: linked::awg_free_string,
                bump_sockets: linked::awg_bump_sockets,
                close: linked::awg_close,
            },
        }
    }

    pub fn from_api(api: AwgApi) -> Self {
        Self { api }
    }

    pub fn api(&self) -> &AwgApi {
        &self.api
    }
}

impl std::fmt::Debug for AwgLib {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AwgLib")
    }
}

/// Коды возврата и уровни журнала из `awg.h`.
pub mod code {
    use std::ffi::c_int;

    pub const LOG_ERROR: c_int = 1;
    pub const LOG_VERBOSE: c_int = 2;

    pub const OK: c_int = 0;
    pub const BAD_HANDLE: c_int = -1000;
    pub const PANIC: c_int = -1001;
    pub const CLOSED: c_int = -1002;
    pub const QUEUE_FULL: c_int = -1003;
    pub const BAD_ARG: c_int = -1004;
    pub const UP: c_int = -1005;
    pub const PROTECT: c_int = -1006;

    /// Человеческое описание кода: errno UAPI (отрицательный) — как есть.
    pub fn describe(code: c_int) -> String {
        match code {
            OK => "ок".into(),
            BAD_HANDLE => "libawg: ядро не найдено (уже закрыто)".into(),
            PANIC => "libawg: паника внутри ядра".into(),
            CLOSED => "libawg: ядро закрывается".into(),
            QUEUE_FULL => "libawg: очередь на шифрование полна, пакет отброшен".into(),
            BAD_ARG => "libawg: неверный аргумент".into(),
            UP => "libawg: сокеты не открылись".into(),
            PROTECT => "libawg: платформа отказалась защитить сокет".into(),
            -22 => "libawg: конфиг отвергнут (EINVAL)".into(),
            -71 => "libawg: кривая строка конфига (EPROTO)".into(),
            -98 => "libawg: порт занят (EADDRINUSE)".into(),
            other => format!("libawg: ошибка {other}"),
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
mod linked {
    use super::AwgCallbacks;
    use std::ffi::{c_char, c_int, c_void};

    unsafe extern "C" {
        pub fn awg_new(
            uapi: *const c_char,
            mtu: c_int,
            cbs: *const AwgCallbacks,
            ctx: *mut c_void,
        ) -> i32;
        pub fn awg_inject(handle: i32, pkt: *const u8, len: usize) -> c_int;
        pub fn awg_set_config(handle: i32, uapi: *const c_char) -> c_int;
        pub fn awg_get_config(handle: i32) -> *mut c_char;
        pub fn awg_free_string(s: *mut c_char);
        pub fn awg_bump_sockets(handle: i32);
        pub fn awg_close(handle: i32);
    }
}
