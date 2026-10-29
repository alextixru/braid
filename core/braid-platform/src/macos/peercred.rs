//! Проверка собеседника unix-сокета по `LOCAL_PEERCRED`: приложение ждёт
//! на той стороне root (помощник), помощник — uid приложения, которое его
//! запустило. Сокет лежит в каталоге 0700 пользователя, проверка — второй
//! рубеж против чужого процесса на том же сокете.

use nix::sys::socket::{getsockopt, sockopt::LocalPeerCred};
use std::io;
use std::os::unix::net::UnixStream;

/// uid процесса на другом конце соединённого unix-сокета.
pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    getsockopt(stream, LocalPeerCred)
        .map(|cred| cred.uid())
        .map_err(io::Error::from)
}
