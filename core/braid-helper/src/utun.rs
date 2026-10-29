// SPDX-FileCopyrightText: 2026 alextixru
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
//
// Источник: `create_utun` из `focsq/backend/tun_darwin.rs` (macOS-порт FOCSQ,
// 2026 alextixru, PolyForm-Noncommercial-1.0.0), без изменений по существу.

//! Создание utun: сокет PF_SYSTEM к `com.apple.net.utun_control`, номер
//! интерфейса (`sc_unit = 0`) выбирает ядро ОС. Требует root
//! (`notes-awg.md` §7: `CTL_FLAG_PRIVILEGED` в XNU).

use anyhow::{Context, Result};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

pub fn create_utun() -> Result<(OwnedFd, String)> {
    const UTUN_CONTROL_NAME: &[u8] = b"com.apple.net.utun_control";
    const UTUN_OPT_IFNAME: libc::c_int = 2;

    // SAFETY: обычный вызов socket без указателей.
    let raw = unsafe { libc::socket(libc::PF_SYSTEM, libc::SOCK_DGRAM, libc::SYSPROTO_CONTROL) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error()).context("socket(PF_SYSTEM)");
    }
    // SAFETY: дескриптор только что создан и ещё ничей.
    let descriptor = unsafe { OwnedFd::from_raw_fd(raw) };

    // SAFETY: ctl_info — plain-данные, нулевое заполнение допустимо.
    let mut info: libc::ctl_info = unsafe { std::mem::zeroed() };
    for (target, source) in info.ctl_name.iter_mut().zip(UTUN_CONTROL_NAME) {
        *target = *source as libc::c_char;
    }
    // SAFETY: fd живой, info — валидная структура нужного размера.
    if unsafe { libc::ioctl(descriptor.as_raw_fd(), libc::CTLIOCGINFO, &mut info) } < 0 {
        return Err(std::io::Error::last_os_error()).context("CTLIOCGINFO utun_control");
    }

    let address = libc::sockaddr_ctl {
        sc_len: std::mem::size_of::<libc::sockaddr_ctl>() as libc::c_uchar,
        sc_family: libc::AF_SYSTEM as libc::c_uchar,
        ss_sysaddr: libc::AF_SYS_CONTROL as u16,
        sc_id: info.ctl_id,
        sc_unit: 0,
        sc_reserved: [0; 5],
    };
    // SAFETY: адрес и его длина согласованы.
    if unsafe {
        libc::connect(
            descriptor.as_raw_fd(),
            (&address as *const libc::sockaddr_ctl).cast(),
            std::mem::size_of::<libc::sockaddr_ctl>() as libc::socklen_t,
        )
    } < 0
    {
        return Err(std::io::Error::last_os_error()).context("создание utun (нужен root)");
    }

    let mut name = [0u8; libc::IFNAMSIZ];
    let mut length = name.len() as libc::socklen_t;
    // SAFETY: буфер и его длина согласованы.
    if unsafe {
        libc::getsockopt(
            descriptor.as_raw_fd(),
            libc::SYSPROTO_CONTROL,
            UTUN_OPT_IFNAME,
            name.as_mut_ptr().cast(),
            &mut length,
        )
    } < 0
    {
        return Err(std::io::Error::last_os_error()).context("UTUN_OPT_IFNAME");
    }
    let end = name
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(length as usize);
    // SAFETY: fd живой.
    if unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error()).context("FD_CLOEXEC на utun");
    }
    Ok((
        descriptor,
        String::from_utf8_lossy(&name[..end]).into_owned(),
    ))
}
